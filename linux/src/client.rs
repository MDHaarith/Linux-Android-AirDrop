//! HTTP client for talking to LocalSend peers.
//!
//! One request per connection. Over HTTPS the peer certificate is pinned to the
//! fingerprint we expect during the TLS handshake, and our own certificate is
//! presented as client certificate.

use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full, Limited, StreamBody};
use hyper::body::Frame;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::identity::{fingerprint, Identity, PIN_MISMATCH};

const MAX_RESPONSE: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum PeerError {
    Connect(String),
    PinMismatch(String),
    Protocol(String),
    Timeout,
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeerError::Connect(e) => write!(f, "Cannot reach the device ({e})"),
            PeerError::PinMismatch(e) => write!(f, "Security check failed: {e}"),
            PeerError::Protocol(e) => write!(f, "Unexpected answer from the device ({e})"),
            PeerError::Timeout => write!(f, "The device did not answer in time"),
        }
    }
}

impl std::error::Error for PeerError {}

pub type ByteStream = Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>;

pub enum Body {
    Empty,
    Json(Vec<u8>),
    Stream {
        len: u64,
        content_type: String,
        stream: ByteStream,
    },
}

impl Body {
    pub fn json(value: &serde_json::Value) -> Self {
        Body::Json(serde_json::to_vec(value).expect("serializable"))
    }
}

#[derive(Clone)]
pub struct Target {
    pub ip: IpAddr,
    pub port: u16,
    pub https: bool,
    /// Pin for HTTPS (None = accept any valid certificate, e.g. while scanning).
    pub fingerprint: Option<String>,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Bytes,
    pub peer_fingerprint: Option<String>,
}

impl Response {
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, PeerError> {
        serde_json::from_slice(&self.body).map_err(|e| PeerError::Protocol(e.to_string()))
    }
}

pub struct Options {
    pub connect_timeout: Duration,
    /// Bound on waiting for the response; None waits forever (prepare-upload
    /// blocks until the receiver accepts).
    pub response_timeout: Option<Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            response_timeout: Some(Duration::from_secs(30)),
        }
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub async fn request(
    identity: &Identity,
    target: &Target,
    method: Method,
    path_and_query: &str,
    body: Body,
    options: Options,
) -> Result<Response, PeerError> {
    let tcp = timeout(
        options.connect_timeout,
        TcpStream::connect((target.ip, target.port)),
    )
    .await
    .map_err(|_| PeerError::Connect("timed out".into()))?
    .map_err(|e| PeerError::Connect(e.to_string()))?;
    let _ = tcp.set_nodelay(true);

    let (io, peer_fingerprint): (Box<dyn Io>, Option<String>) = if target.https {
        let connector = TlsConnector::from(identity.client_config(target.fingerprint.as_deref()));
        let server_name = ServerName::IpAddress(target.ip.into());
        let tls = timeout(options.connect_timeout, connector.connect(server_name, tcp))
            .await
            .map_err(|_| PeerError::Connect("TLS handshake timed out".into()))?
            .map_err(|e| {
                let text = e.to_string();
                if text.contains(PIN_MISMATCH) {
                    PeerError::PinMismatch(text)
                } else {
                    PeerError::Connect(text)
                }
            })?;
        let fp = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|c| fingerprint(c));
        (Box::new(tls), fp)
    } else {
        (Box::new(tcp), None)
    };

    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|e| PeerError::Connect(e.to_string()))?;
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let host = format!("{}:{}", target.ip, target.port);
    let builder = Request::builder()
        .method(method)
        .uri(path_and_query)
        .header("host", host)
        .header(
            "user-agent",
            concat!("AirBridge/", env!("CARGO_PKG_VERSION")),
        )
        .header("accept", "*/*");
    let request = match body {
        Body::Empty => builder
            .header("content-length", "0")
            .body(UnsyncBoxBody::new(
                Full::new(Bytes::new()).map_err(|never| match never {}),
            )),
        Body::Json(bytes) => builder
            .header("content-type", "application/json")
            .header("content-length", bytes.len().to_string())
            .body(UnsyncBoxBody::new(
                Full::new(Bytes::from(bytes)).map_err(|never| match never {}),
            )),
        Body::Stream {
            len,
            content_type,
            stream,
        } => builder
            .header("content-type", content_type)
            .header("content-length", len.to_string())
            .body(UnsyncBoxBody::new(StreamBody::new(
                stream.map_ok(Frame::data),
            ))),
    }
    .map_err(|e| PeerError::Protocol(e.to_string()))?;

    let exchange = async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| PeerError::Connect(e.to_string()))?;
        let status = response.status().as_u16();
        let body = Limited::new(response.into_body(), MAX_RESPONSE)
            .collect()
            .await
            .map_err(|e| PeerError::Protocol(e.to_string()))?
            .to_bytes();
        Ok(Response {
            status,
            body,
            peer_fingerprint,
        })
    };
    match options.response_timeout {
        Some(limit) => timeout(limit, exchange)
            .await
            .map_err(|_| PeerError::Timeout)?,
        None => exchange.await,
    }
}

/// Wraps a byte stream so that `on_progress` sees the running total.
pub fn with_progress(
    stream: ByteStream,
    on_progress: Arc<dyn Fn(u64) + Send + Sync>,
) -> ByteStream {
    let mut sent = 0u64;
    Box::pin(stream.inspect(move |chunk| {
        if let Ok(chunk) = chunk {
            sent += chunk.len() as u64;
            on_progress(sent);
        }
    }))
}
