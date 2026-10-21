//! UKEY2 key exchange and the SecureMessage channel Quick Share runs on.
//!
//! Sources: the google/ukey2 README (handshake and key derivation),
//! Chromium's `d2d_crypto_ops.cc` / `crypto_ops.cc` (session keys) and
//! NearDrop's PROTOCOL.md (how Nearby Share uses all of it).

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use anyhow::{anyhow, bail, Result};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use prost::Message;
use rand::RngCore;
use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, ECDH_P256};
use sha2::{Digest, Sha256, Sha512};

use super::proto::securegcm::{
    self, ukey2_alert::AlertType, ukey2_client_init::CipherCommitment, ukey2_message,
    DeviceToDeviceMessage, GcmMetadata, Ukey2Alert, Ukey2ClientFinished, Ukey2ClientInit,
    Ukey2HandshakeCipher, Ukey2Message, Ukey2ServerInit,
};
use super::proto::securemessage::{
    EcP256PublicKey, EncScheme, GenericPublicKey, Header, HeaderAndBody, PublicKeyType,
    SecureMessage, SigScheme,
};

const NEXT_PROTOCOL: &str = "AES_256_CBC-HMAC_SHA256";

/// A handshake step the peer got wrong, with the alert UKEY2 wants us to send.
#[derive(Debug)]
pub struct Ukey2Error {
    pub alert: AlertType,
    pub reason: &'static str,
}

impl std::fmt::Display for Ukey2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UKEY2 handshake failed: {}", self.reason)
    }
}

impl std::error::Error for Ukey2Error {}

fn fail(alert: AlertType, reason: &'static str) -> Ukey2Error {
    Ukey2Error { alert, reason }
}

/// The serialized alert to send before hanging up.
pub fn alert_message(error: &Ukey2Error) -> Vec<u8> {
    Ukey2Message {
        message_type: Some(ukey2_message::Type::Alert as i32),
        message_data: Some(
            Ukey2Alert {
                r#type: Some(error.alert as i32),
                error_message: Some(error.reason.into()),
            }
            .encode_to_vec(),
        ),
    }
    .encode_to_vec()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn new_key_pair() -> (EphemeralPrivateKey, Vec<u8>) {
    let rng = ring::rand::SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng).expect("system RNG works");
    let public = private
        .compute_public_key()
        .expect("public key of a fresh key pair")
        .as_ref()
        .to_vec();
    (private, public)
}

/// Java's `BigInteger.toByteArray()`: big-endian two's complement, minimal.
fn to_signed_bytes(magnitude: &[u8]) -> Vec<u8> {
    let start = magnitude
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(magnitude.len());
    let trimmed = &magnitude[start..];
    match trimmed.first() {
        None => vec![0],
        Some(&b) if b & 0x80 != 0 => [&[0u8][..], trimmed].concat(),
        Some(_) => trimmed.to_vec(),
    }
}

/// Accepts minimal two's complement as well as plain 32-byte coordinates.
fn to_coordinate(bytes: &[u8]) -> Option<[u8; 32]> {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let trimmed = &bytes[start..];
    if trimmed.len() > 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out[32 - trimmed.len()..].copy_from_slice(trimmed);
    Some(out)
}

/// `GenericPublicKey` for an uncompressed P-256 point (0x04 ‖ x ‖ y).
fn encode_public_key(uncompressed: &[u8]) -> Vec<u8> {
    GenericPublicKey {
        r#type: PublicKeyType::EcP256 as i32,
        ec_p256_public_key: Some(EcP256PublicKey {
            x: to_signed_bytes(&uncompressed[1..33]),
            y: to_signed_bytes(&uncompressed[33..65]),
        }),
        ..Default::default()
    }
    .encode_to_vec()
}

fn decode_public_key(bytes: &[u8]) -> Option<[u8; 65]> {
    let key = GenericPublicKey::decode(bytes).ok()?;
    if key.r#type != PublicKeyType::EcP256 as i32 {
        return None;
    }
    let point = key.ec_p256_public_key?;
    let mut out = [0u8; 65];
    out[0] = 0x04;
    out[1..33].copy_from_slice(&to_coordinate(&point.x)?);
    out[33..].copy_from_slice(&to_coordinate(&point.y)?);
    Some(out)
}

fn ecdh(private: EphemeralPrivateKey, peer: &[u8; 65]) -> Option<[u8; 32]> {
    agreement::agree_ephemeral(
        private,
        &UnparsedPublicKey::new(&ECDH_P256, peer),
        |shared| {
            let mut x = [0u8; 32];
            x.copy_from_slice(shared);
            x
        },
    )
    .ok()
}

fn ukey2_message(kind: ukey2_message::Type, data: Vec<u8>) -> Vec<u8> {
    Ukey2Message {
        message_type: Some(kind as i32),
        message_data: Some(data),
    }
    .encode_to_vec()
}

fn parse_ukey2(raw: &[u8], expected: ukey2_message::Type) -> Result<Vec<u8>, Ukey2Error> {
    let message =
        Ukey2Message::decode(raw).map_err(|_| fail(AlertType::BadMessage, "malformed message"))?;
    if message.message_type != Some(expected as i32) {
        return Err(fail(AlertType::BadMessageType, "unexpected message type"));
    }
    message
        .message_data
        .ok_or(fail(AlertType::BadMessageData, "missing message data"))
}

/// Server side: we received a ClientInit and answer with a ServerInit.
pub struct ServerHandshake {
    client_init: Vec<u8>,
    server_init: Vec<u8>,
    commitment: Vec<u8>,
    private: EphemeralPrivateKey,
}

impl ServerHandshake {
    /// Returns the handshake state and the ServerInit to send.
    pub fn new(client_init_raw: &[u8]) -> Result<(Self, Vec<u8>), Ukey2Error> {
        let data = parse_ukey2(client_init_raw, ukey2_message::Type::ClientInit)?;
        let init = Ukey2ClientInit::decode(data.as_slice())
            .map_err(|_| fail(AlertType::BadMessageData, "malformed ClientInit"))?;
        if init.version != Some(1) {
            return Err(fail(AlertType::BadVersion, "unsupported version"));
        }
        if init.random.as_ref().map(Vec::len) != Some(32) {
            return Err(fail(AlertType::BadRandom, "bad random"));
        }
        let commitment = init
            .cipher_commitments
            .iter()
            .find(|c| c.handshake_cipher == Some(Ukey2HandshakeCipher::P256Sha512 as i32))
            .and_then(|c| c.commitment.clone())
            .ok_or(fail(AlertType::BadHandshakeCipher, "no P-256 commitment"))?;
        if init.next_protocol.as_deref() != Some(NEXT_PROTOCOL) {
            return Err(fail(
                AlertType::BadNextProtocol,
                "unsupported next protocol",
            ));
        }

        let (private, public) = new_key_pair();
        let server_init = ukey2_message(
            ukey2_message::Type::ServerInit,
            Ukey2ServerInit {
                version: Some(1),
                random: Some(random_bytes::<32>().to_vec()),
                handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512 as i32),
                public_key: Some(encode_public_key(&public)),
            }
            .encode_to_vec(),
        );
        Ok((
            Self {
                client_init: client_init_raw.to_vec(),
                server_init: server_init.clone(),
                commitment,
                private,
            },
            server_init,
        ))
    }

    /// Verifies the ClientFinish against the commitment and derives the keys.
    pub fn finish(self, client_finish_raw: &[u8]) -> Result<Session, Ukey2Error> {
        if Sha512::digest(client_finish_raw).as_slice() != self.commitment.as_slice() {
            return Err(fail(AlertType::BadMessage, "commitment mismatch"));
        }
        let data = parse_ukey2(client_finish_raw, ukey2_message::Type::ClientFinish)?;
        let finish = Ukey2ClientFinished::decode(data.as_slice())
            .map_err(|_| fail(AlertType::BadMessageData, "malformed ClientFinish"))?;
        let peer = finish
            .public_key
            .as_deref()
            .and_then(decode_public_key)
            .ok_or(fail(AlertType::BadPublicKey, "bad public key"))?;
        let shared =
            ecdh(self.private, &peer).ok_or(fail(AlertType::BadPublicKey, "invalid point"))?;
        Ok(Session::derive(
            &shared,
            &self.client_init,
            &self.server_init,
            true,
        ))
    }
}

/// Client side: we send ClientInit, then ClientFinish after the ServerInit.
pub struct ClientHandshake {
    client_init: Vec<u8>,
    client_finish: Vec<u8>,
    private: EphemeralPrivateKey,
}

impl ClientHandshake {
    pub fn new() -> Self {
        let (private, public) = new_key_pair();
        let client_finish = ukey2_message(
            ukey2_message::Type::ClientFinish,
            Ukey2ClientFinished {
                public_key: Some(encode_public_key(&public)),
            }
            .encode_to_vec(),
        );
        let client_init = ukey2_message(
            ukey2_message::Type::ClientInit,
            Ukey2ClientInit {
                version: Some(1),
                random: Some(random_bytes::<32>().to_vec()),
                cipher_commitments: vec![CipherCommitment {
                    handshake_cipher: Some(Ukey2HandshakeCipher::P256Sha512 as i32),
                    commitment: Some(Sha512::digest(&client_finish).to_vec()),
                }],
                next_protocol: Some(NEXT_PROTOCOL.into()),
            }
            .encode_to_vec(),
        );
        Self {
            client_init,
            client_finish,
            private,
        }
    }

    pub fn client_init(&self) -> &[u8] {
        &self.client_init
    }

    /// Returns the session and the ClientFinish to send.
    pub fn finish(self, server_init_raw: &[u8]) -> Result<(Session, Vec<u8>), Ukey2Error> {
        let data = parse_ukey2(server_init_raw, ukey2_message::Type::ServerInit)?;
        let init = Ukey2ServerInit::decode(data.as_slice())
            .map_err(|_| fail(AlertType::BadMessageData, "malformed ServerInit"))?;
        if init.version != Some(1) {
            return Err(fail(AlertType::BadVersion, "unsupported version"));
        }
        if init.random.as_ref().map(Vec::len) != Some(32) {
            return Err(fail(AlertType::BadRandom, "bad random"));
        }
        if init.handshake_cipher != Some(Ukey2HandshakeCipher::P256Sha512 as i32) {
            return Err(fail(AlertType::BadHandshakeCipher, "unexpected cipher"));
        }
        let peer = init
            .public_key
            .as_deref()
            .and_then(decode_public_key)
            .ok_or(fail(AlertType::BadPublicKey, "bad public key"))?;
        let shared =
            ecdh(self.private, &peer).ok_or(fail(AlertType::BadPublicKey, "invalid point"))?;
        let session = Session::derive(&shared, &self.client_init, server_init_raw, false);
        Ok((session, self.client_finish))
    }
}

fn hkdf32(ikm: &[u8], salt: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, &mut out)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    out
}

/// The result of a completed handshake.
pub struct Session {
    /// Four digits both screens show so people can check they are connected
    /// to each other.
    pub pin: String,
    pub sealer: Sealer,
    pub opener: Opener,
}

impl Session {
    fn derive(shared_x: &[u8; 32], client_init: &[u8], server_init: &[u8], server: bool) -> Self {
        let (auth, next) = ukey2_secrets(shared_x, client_init, server_init);
        let keys = session_keys(&next);
        let (send, receive) = if server {
            (
                (keys.server_enc, keys.server_sig),
                (keys.client_enc, keys.client_sig),
            )
        } else {
            (
                (keys.client_enc, keys.client_sig),
                (keys.server_enc, keys.server_sig),
            )
        };
        Self {
            pin: pin_code(&auth),
            sealer: Sealer {
                key: send.0,
                hmac: send.1,
                sequence: 0,
            },
            opener: Opener {
                key: receive.0,
                hmac: receive.1,
                sequence: 0,
            },
        }
    }
}

/// (authentication string, next protocol secret) from the UKEY2 README.
fn ukey2_secrets(
    shared_x: &[u8; 32],
    client_init: &[u8],
    server_init: &[u8],
) -> ([u8; 32], [u8; 32]) {
    let secret = Sha256::digest(shared_x);
    let info = [client_init, server_init].concat();
    (
        hkdf32(&secret, b"UKEY2 v1 auth", &info),
        hkdf32(&secret, b"UKEY2 v1 next", &info),
    )
}

struct SessionKeys {
    client_enc: [u8; 32],
    client_sig: [u8; 32],
    server_enc: [u8; 32],
    server_sig: [u8; 32],
}

fn session_keys(next_secret: &[u8; 32]) -> SessionKeys {
    let d2d_salt = Sha256::digest(b"D2D");
    let client = hkdf32(next_secret, &d2d_salt, b"client");
    let server = hkdf32(next_secret, &d2d_salt, b"server");
    let salt = Sha256::digest(b"SecureMessage");
    SessionKeys {
        client_enc: hkdf32(&client, &salt, b"ENC:2"),
        client_sig: hkdf32(&client, &salt, b"SIG:1"),
        server_enc: hkdf32(&server, &salt, b"ENC:2"),
        server_sig: hkdf32(&server, &salt, b"SIG:1"),
    }
}

/// Chromium's `ToFourDigitString`: a hash of the auth string, mod 9973.
pub fn pin_code(auth: &[u8]) -> String {
    const MODULO: i32 = 9973;
    let mut hash: i32 = 0;
    let mut multiplier: i32 = 1;
    for &byte in auth {
        hash = (hash + i32::from(byte as i8) * multiplier) % MODULO;
        multiplier = (multiplier * 31) % MODULO;
    }
    format!("{:04}", hash.abs())
}

type HmacSha256 = Hmac<Sha256>;

/// Encrypts and signs outgoing offline frames:
/// SecureMessage(HeaderAndBody(AES-256-CBC(DeviceToDeviceMessage))) + HMAC.
pub struct Sealer {
    key: [u8; 32],
    hmac: [u8; 32],
    sequence: i32,
}

impl Sealer {
    /// Wraps a serialized offline frame into a serialized SecureMessage.
    pub fn seal(&mut self, frame: &[u8]) -> Vec<u8> {
        self.sequence += 1;
        let plain = DeviceToDeviceMessage {
            message: Some(frame.to_vec()),
            sequence_number: Some(self.sequence),
        }
        .encode_to_vec();
        let iv = random_bytes::<16>();
        let body = cbc::Encryptor::<aes::Aes256>::new(&self.key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        let header_and_body = HeaderAndBody {
            header: Header {
                signature_scheme: SigScheme::HmacSha256 as i32,
                encryption_scheme: EncScheme::Aes256Cbc as i32,
                iv: Some(iv.to_vec()),
                public_metadata: Some(
                    GcmMetadata {
                        r#type: securegcm::Type::DeviceToDeviceMessage as i32,
                        version: Some(1),
                    }
                    .encode_to_vec(),
                ),
                ..Default::default()
            },
            body,
        }
        .encode_to_vec();
        let mut mac = HmacSha256::new_from_slice(&self.hmac).expect("any key length");
        mac.update(&header_and_body);
        SecureMessage {
            header_and_body,
            signature: mac.finalize().into_bytes().to_vec(),
        }
        .encode_to_vec()
    }
}

/// Verifies and decrypts incoming SecureMessages, in order.
pub struct Opener {
    key: [u8; 32],
    hmac: [u8; 32],
    sequence: i32,
}

impl Opener {
    /// Returns the serialized offline frame inside a SecureMessage.
    pub fn open(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        let message = SecureMessage::decode(message)?;
        let mut mac = HmacSha256::new_from_slice(&self.hmac).expect("any key length");
        mac.update(&message.header_and_body);
        mac.verify_slice(&message.signature)
            .map_err(|_| anyhow!("bad message signature"))?;
        let header_and_body = HeaderAndBody::decode(message.header_and_body.as_slice())?;
        let iv: [u8; 16] = header_and_body
            .header
            .iv
            .as_deref()
            .and_then(|iv| iv.try_into().ok())
            .ok_or_else(|| anyhow!("missing IV"))?;
        let plain = cbc::Decryptor::<aes::Aes256>::new(&self.key.into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&header_and_body.body)
            .map_err(|_| anyhow!("bad padding"))?;
        let d2d = DeviceToDeviceMessage::decode(plain.as_slice())?;
        let expected = self.sequence + 1;
        if d2d.sequence_number != Some(expected) {
            bail!(
                "unexpected sequence number {:?} (wanted {expected})",
                d2d.sequence_number
            );
        }
        self.sequence = expected;
        d2d.message.ok_or_else(|| anyhow!("empty message"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn salts_match_the_published_constants() {
        assert_eq!(
            hex(&Sha256::digest(b"D2D")),
            "82aa55a0d397f88346ca1cee8d3909b95f13fa7deb1d4ab38376b8256da85510"
        );
        assert_eq!(
            hex(&Sha256::digest(b"SecureMessage")),
            "bf9d2a53c63616d75db0a7165b91c1ef73e537f2427405fa23610a4be657642e"
        );
    }

    #[test]
    fn signed_coordinates_round_trip() {
        let mut high = [0u8; 32];
        high[0] = 0x80;
        high[31] = 7;
        assert_eq!(to_signed_bytes(&high).len(), 33);
        assert_eq!(to_coordinate(&to_signed_bytes(&high)), Some(high));
        let mut low = [0u8; 32];
        low[1] = 0x42;
        assert_eq!(to_signed_bytes(&low).len(), 31);
        assert_eq!(to_coordinate(&to_signed_bytes(&low)), Some(low));
        // Raw 32-byte coordinates with the high bit set are accepted too.
        assert_eq!(to_coordinate(&high), Some(high));
        assert_eq!(to_coordinate(&[1u8; 33]), None);
    }

    #[test]
    fn pin_is_four_digits_and_uses_signed_bytes() {
        assert_eq!(pin_code(&[0u8; 32]), "0000");
        // 1 + 31·(-1) = -30 → "0030"
        assert_eq!(pin_code(&[1, 0xff]), "0030");
        for seed in 0..50u8 {
            let auth: Vec<u8> = (0..32u8)
                .map(|i| seed.wrapping_mul(37).wrapping_add(i.wrapping_mul(11)))
                .collect();
            let pin = pin_code(&auth);
            assert_eq!(pin.len(), 4);
            assert!(pin.parse::<u32>().unwrap() < 9973);
        }
    }

    #[test]
    fn handshake_agrees_on_keys_and_pin() {
        let client = ClientHandshake::new();
        let (server, server_init) = ServerHandshake::new(client.client_init()).unwrap();
        let (mut client_session, client_finish) = client.finish(&server_init).unwrap();
        let mut server_session = server.finish(&client_finish).unwrap();
        assert_eq!(client_session.pin, server_session.pin);

        for i in 0..3 {
            let text = format!("frame {i}");
            let sealed = client_session.sealer.seal(text.as_bytes());
            assert_eq!(
                server_session.opener.open(&sealed).unwrap(),
                text.as_bytes()
            );
        }
        let sealed = server_session.sealer.seal(b"reply");
        assert_eq!(client_session.opener.open(&sealed).unwrap(), b"reply");
    }

    #[test]
    fn tampering_replay_and_bad_commitments_are_rejected() {
        let client = ClientHandshake::new();
        let (server, server_init) = ServerHandshake::new(client.client_init()).unwrap();
        let (mut client_session, client_finish) = client.finish(&server_init).unwrap();
        let mut server_session = server.finish(&client_finish).unwrap();

        let sealed = client_session.sealer.seal(b"hello");
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(server_session.opener.open(&tampered).is_err());
        server_session.opener.open(&sealed).unwrap();
        assert!(server_session.opener.open(&sealed).is_err(), "replay");

        // A ClientFinish that does not match the ClientInit commitment.
        let client = ClientHandshake::new();
        let other = ClientHandshake::new();
        let (server, server_init) = ServerHandshake::new(client.client_init()).unwrap();
        let (_, wrong_finish) = other.finish(&server_init).unwrap();
        let error = server.finish(&wrong_finish).err().unwrap();
        assert_eq!(error.alert, AlertType::BadMessage);
    }

    #[test]
    fn client_init_is_validated() {
        let bad = ukey2_message(
            ukey2_message::Type::ClientInit,
            Ukey2ClientInit {
                version: Some(1),
                random: Some(vec![0; 32]),
                cipher_commitments: vec![],
                next_protocol: Some(NEXT_PROTOCOL.into()),
            }
            .encode_to_vec(),
        );
        let error = ServerHandshake::new(&bad).err().unwrap();
        assert_eq!(error.alert, AlertType::BadHandshakeCipher);
        let error = ServerHandshake::new(b"\xff\xff").err().unwrap();
        assert_eq!(error.alert, AlertType::BadMessage);
    }

    /// Fixed inputs → fixed keys. The expected values come from
    /// `tools/quickshare_kat.py`, an independent Python implementation.
    #[test]
    fn derivation_known_answer() {
        let (auth, next) = ukey2_secrets(&[7u8; 32], b"client-init", b"server-init");
        let keys = session_keys(&next);
        assert_eq!(hex(&auth), KAT_AUTH);
        assert_eq!(hex(&next), KAT_NEXT);
        assert_eq!(hex(&keys.client_enc), KAT_CLIENT_ENC);
        assert_eq!(hex(&keys.server_sig), KAT_SERVER_SIG);
        assert_eq!(pin_code(&auth), KAT_PIN);
    }

    const KAT_AUTH: &str = "9df251948f7975a7775eb98e7db0b0b630c93f1987b96184025cbeffc9b224e1";
    const KAT_NEXT: &str = "d6d3953141a0c0913f060127718b1ab4b23c2b15cd15c396dc6f0b562c730073";
    const KAT_CLIENT_ENC: &str = "8619d27056518d85ed8b65f556ea485ffa9dd646d5ddd1716c9b4d7650afdeb7";
    const KAT_SERVER_SIG: &str = "edf1486c3a9656b7029cf9d0e998811778b43e8ada8e076b8c68c69d9162b67c";
    const KAT_PIN: &str = "7252";
}
