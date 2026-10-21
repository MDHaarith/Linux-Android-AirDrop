//! Device identity: a self-signed TLS certificate and its fingerprint.
//!
//! LocalSend identifies a device by the uppercase hex SHA-256 of its
//! certificate (DER). The same certificate is our HTTPS server certificate and
//! our TLS client certificate: LocalSend peers require a client certificate and
//! check that its fingerprint matches the one we announce.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, OtherError, ServerConfig,
    SignatureScheme,
};
use sha2::{Digest, Sha256};

use crate::config::write_private;

/// Marker text used to recognize pinning failures in connection errors.
pub const PIN_MISMATCH: &str = "certificate fingerprint mismatch";

pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect()
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts any certificate that is well-formed, currently valid and correctly
/// self-signed. Identity comes from the fingerprint, not from a CA.
fn check_self_signed(der: &[u8]) -> Result<(), Error> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der)
        .map_err(|_| Error::InvalidCertificate(CertificateError::BadEncoding))?;
    if !cert.validity().is_valid() {
        return Err(Error::InvalidCertificate(CertificateError::Expired));
    }
    cert.verify_signature(None)
        .map_err(|_| Error::InvalidCertificate(CertificateError::BadSignature))
}

#[derive(Debug)]
struct PinMismatch(String);

impl fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PIN_MISMATCH}: {}", self.0)
    }
}

impl std::error::Error for PinMismatch {}

/// Server verifier used by our client: optional fingerprint pin, no CA chain.
#[derive(Debug)]
struct PinnedServerVerifier {
    expected: Option<String>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        check_self_signed(end_entity)?;
        if let Some(expected) = &self.expected {
            let actual = fingerprint(end_entity);
            if &actual != expected {
                return Err(Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(
                    PinMismatch(format!("expected {}…, got {}…", &expected[..12.min(expected.len())], &actual[..12])),
                )))));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Client verifier used by our server: asks for (but does not require) a
/// client certificate, so we learn the sender's real fingerprint.
#[derive(Debug)]
struct AnyClientCert {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyClientCert {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        check_self_signed(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

pub struct Identity {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    pub fingerprint: String,
    server_config: Arc<ServerConfig>,
}

impl Identity {
    /// Loads the identity from `dir`, generating it on first start.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        if !cert_path.exists() || !key_path.exists() {
            tracing::info!("Generating device certificate (one-time)…");
            generate(&cert_path, &key_path)?;
        }
        let cert = CertificateDer::from_pem_file(&cert_path)
            .with_context(|| format!("reading {}", cert_path.display()))?;
        let key = PrivateKeyDer::from_pem_file(&key_path)
            .with_context(|| format!("reading {}", key_path.display()))?;
        let fingerprint = fingerprint(&cert);

        let algorithms = provider().signature_verification_algorithms;
        let mut server_config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(Arc::new(AnyClientCert { algorithms }))
            .with_single_cert(vec![cert.clone()], key.clone_key())?;
        server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

        Ok(Self { cert, key, fingerprint, server_config: Arc::new(server_config) })
    }

    pub fn server_config(&self) -> Arc<ServerConfig> {
        self.server_config.clone()
    }

    /// A client config presenting our certificate and pinning the peer's
    /// fingerprint (if known). The check runs during the handshake, before any
    /// request byte is sent.
    pub fn client_config(&self, expected_fingerprint: Option<&str>) -> Arc<ClientConfig> {
        let verifier = PinnedServerVerifier {
            expected: expected_fingerprint.map(|f| f.to_ascii_uppercase()),
            algorithms: provider().signature_verification_algorithms,
        };
        let mut config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_client_auth_cert(vec![self.cert.clone()], self.key.clone_key())
            .expect("our own certificate and key match");
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(config)
    }
}

fn generate(cert_path: &Path, key_path: &Path) -> Result<()> {
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};

    // RSA-2048, like the official LocalSend apps, for maximum compatibility.
    let private_key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048)?;
    let key_pem = private_key.to_pkcs8_pem(LineEnding::LF)?;
    let key_pair = rcgen::KeyPair::try_from(private_key.to_pkcs8_der()?.as_bytes())?;

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, "AirBridge User");
    let cert = params.self_signed(&key_pair)?;

    write_private(key_path, key_pem.as_bytes())?;
    std::fs::write(cert_path, cert.pem())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_matches_localsend_format() {
        // Test vector from LocalSend's core/src/crypto/cert.rs.
        const CERT: &str = "-----BEGIN CERTIFICATE-----
MIIDGTCCAgGgAwIBAgIBATANBgkqhkiG9w0BAQsFADBQMRcwFQYDVQQDEw5Mb2Nh
bFNlbmQgVXNlcjEJMAcGA1UEChMAMQkwBwYDVQQLEwAxCTAHBgNVBAcTADEJMAcG
A1UECBMAMQkwBwYDVQQGEwAwHhcNMjUwMjA5MDAwMzE0WhcNMzUwMjA3MDAwMzE0
WjBQMRcwFQYDVQQDEw5Mb2NhbFNlbmQgVXNlcjEJMAcGA1UEChMAMQkwBwYDVQQL
EwAxCTAHBgNVBAcTADEJMAcGA1UECBMAMQkwBwYDVQQGEwAwggEiMA0GCSqGSIb3
DQEBAQUAA4IBDwAwggEKAoIBAQCL24MxhGfrdJm0Q8ZGiBkZ27ldcEChB4w7rSbJ
yiKeosoNbJl2kyj5dZjfBhWGgDLGDMM5w+Mh/5SrWgTL/QrhbB+lsrxILLznWqBi
R8wJP0P2YW9fBahQskJQcUXt/3jsCsMTWea4rWc3HZGh03bAkJfLM+PDSOfTpvAZ
6DQSp9QLzC9bgVNnq3W0SvOZGpF0xRa4InCyTUgxsNsV4+GIrmN5w4EbRFVVYu7D
5OS5fxNSCukiS0fb6oQzUp0vIAycvvWHHbAy8T6UMoUor2nfvNcryiaOX5WBMLyh
yMZ5gMOyXjdm1bT1XSlvtXPYUzxvsGAzTqS8mXjw8h7mm5htAgMBAAEwDQYJKoZI
hvcNAQELBQADggEBABZ+I7D6wkeSrsi1NBLP2zoZ5oGh+INNcGTravfOQHs4Fbas
/CysaUYjsD3fmaDh4MxgWEqAmWnnBiojfpGX2SGuFqRBKyT9DgitBt0L7Ezg1k3h
bfSiFW4hXWp75grVO8xfML7ZcWMlhKrOsOMUGiy1qs3qsyJ3w7B2Tz78HhXGO5dd
jyPmZarhixKO92UpEvKGxjO0E/3UUNUzxKTAAgFfhKpuwHUgIijM/EppZtA8OcSh
fEztiV0xKfcPVx4d6dqRt/NMElK1Ivw2vUuxTymphZkkFOzht9m73/kyKaeFp8Ij
VRus1zGVD8IVpIdPMyz01WJyS7M0fWaHXKWo+Bo=
-----END CERTIFICATE-----";
        let der = CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap();
        assert_eq!(fingerprint(&der), "4BADDE53A7F7CDEEED93189FD898E02BF6B4806CA4C05DE0ACE08319B86552FA");
        check_self_signed(&der).unwrap();
    }

    #[test]
    fn generated_identity_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(identity.fingerprint.len(), 64);
        check_self_signed(&identity.cert).unwrap();
        // Loading again yields the same identity.
        let again = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(identity.fingerprint, again.fingerprint);
    }
}
