//! The LocalSend v2 HTTPS API: the part other devices talk to.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use bytes::Bytes;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rand::distributions::{Alphanumeric, DistString};
use rustls::ServerConfig;
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

use crate::app::App;
use crate::identity::fingerprint;
use crate::limits::ConnLimit;
use crate::net;
use crate::protocol::{is_localsend_id, DeviceInfo, FileDto, PrepareUploadRequest, API, MAX_FILES};
use crate::storage::{self, SaveError};
use crate::transfers::{now, Direction, FileState, FileStatus, Status, Transfer};

/// Request bodies of register and prepare-upload (file lists, text).
const MAX_BODY: usize = 4 * 1024 * 1024;
/// Time a client gets to send a request's headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);

/// Who is on the other end of a connection.
#[derive(Clone, Debug)]
pub struct ConnInfo {
    pub ip: IpAddr,
    /// Fingerprint of the TLS client certificate, if one was presented.
    pub cert_fingerprint: Option<String>,
}

fn error(status: u16, message: &str) -> Response {
    tracing::debug!("answering {status}: {message}");
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({ "message": message }))).into_response()
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(&format!("{API}/register"), post(register))
        .route(&format!("{API}/info"), get(info))
        .route("/api/localsend/v1/info", get(info))
        .route(&format!("{API}/prepare-upload"), post(prepare_upload))
        .route(&format!("{API}/upload"), post(upload))
        .route(&format!("{API}/cancel"), post(cancel))
        .route("/api/airbridge/v1/pair", post(pair))
        .route("/", get(|| async { "AirBridge (LocalSend protocol v2)\n" }))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(app)
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Accept loop: TLS handshake (learning the client certificate), then HTTP.
pub async fn serve(app: Arc<App>, listener: TcpListener, tls: Option<Arc<ServerConfig>>) {
    let router = router(app);
    let acceptor = tls.map(TlsAcceptor::from);
    // Enough for many devices at once, not for a flood of idle connections.
    let limits = ConnLimit::new(128, 16);
    loop {
        let (tcp, addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!("accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let ip = net::canonical(addr.ip());
        let Some(permit) = limits.try_acquire(ip) else {
            tracing::debug!("too many connections from {ip}");
            continue;
        };
        let (router, acceptor) = (router.clone(), acceptor.clone());
        tokio::spawn(async move {
            let _permit = permit;
            let (io, cert_fingerprint): (Box<dyn Io>, Option<String>) = match acceptor {
                Some(acceptor) => {
                    match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(tcp)).await
                    {
                        Ok(Ok(stream)) => {
                            let fp = stream
                                .get_ref()
                                .1
                                .peer_certificates()
                                .and_then(|c| c.first())
                                .map(|c| fingerprint(c));
                            (Box::new(stream), fp)
                        }
                        Ok(Err(err)) => {
                            tracing::debug!("TLS handshake with {ip} failed: {err}");
                            return;
                        }
                        Err(_) => return,
                    }
                }
                None => (Box::new(tcp), None),
            };
            let conn = ConnInfo {
                ip,
                cert_fingerprint,
            };
            let service = router.map_request(move |mut request: Request<Incoming>| {
                request.extensions_mut().insert(conn.clone());
                request
            });
            // HTTP/1.1 only (as negotiated), with a deadline for headers so
            // half-open connections do not pile up.
            let mut builder = auto::Builder::new(TokioExecutor::new()).http1_only();
            builder
                .http1()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT);
            if let Err(err) = builder
                .serve_connection(TokioIo::new(io), TowerToHyperService::new(service))
                .await
            {
                tracing::debug!("connection with {ip} ended: {err}");
            }
        });
    }
}

/// The sender's device info, and whether its id is proven by its TLS client
/// certificate (true) or merely claimed (false; e.g. the official LocalSend
/// apps send no certificate). Only proven ids may match trusted devices.
fn parse_info(app: &App, conn: &ConnInfo, info: DeviceInfo) -> Option<(DeviceInfo, bool)> {
    let mut info = info.sanitized().ok()?;
    if let Some(fp) = &conn.cert_fingerprint {
        // The certificate proves who they are; a different claim is spoofed.
        if !info.fingerprint.is_empty() && !info.fingerprint.eq_ignore_ascii_case(fp) {
            tracing::warn!(
                "{}: claimed fingerprint does not match its certificate",
                conn.ip
            );
            return None;
        }
        info.fingerprint = fp.clone();
    }
    if !is_localsend_id(&info.fingerprint)
        || info
            .fingerprint
            .eq_ignore_ascii_case(&app.identity.fingerprint)
    {
        return None;
    }
    let verified = conn.cert_fingerprint.is_some();
    // Without a certificate, the id of one of your devices (or of any device
    // that proved its id before) cannot be claimed: it would show up as that
    // device in the prompt.
    if !verified
        && (app.registry.is_trusted(&info.fingerprint)
            || app
                .registry
                .get(&info.fingerprint)
                .is_some_and(|p| p.verified))
    {
        tracing::warn!(
            "{}: claims to be {} without its certificate",
            conn.ip,
            info.alias
        );
        return None;
    }
    Some((info, verified))
}

async fn info(State(app): State<Arc<App>>) -> Response {
    Json(app.me().info_json()).into_response()
}

async fn register(
    State(app): State<Arc<App>>,
    Extension(conn): Extension<ConnInfo>,
    body: Bytes,
) -> Response {
    let Ok(info) = serde_json::from_slice::<DeviceInfo>(&body) else {
        return error(400, "Invalid body");
    };
    if let Some((info, verified)) = parse_info(&app, &conn, info) {
        app.upsert_peer(info, conn.ip, verified);
    }
    Json(app.me().info_json()).into_response()
}

#[derive(Debug, Deserialize)]
struct PairRequest {
    alias: String,
    port: u16,
    addresses: Vec<Ipv4Addr>,
}

impl PairRequest {
    fn parse(body: &[u8]) -> Result<Option<Self>, &'static str> {
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        let mut request: Self = serde_json::from_slice(body).map_err(|_| "Invalid pair body")?;
        if request.port < 1024
            || request.addresses.len() > 8
            || request
                .addresses
                .iter()
                .any(|ip| !net::is_pair_address(*ip))
        {
            return Err("Invalid pair addresses or port");
        }
        let info = DeviceInfo {
            alias: request.alias,
            version: String::new(),
            device_model: None,
            device_type: None,
            fingerprint: String::new(),
            port: request.port,
            protocol: "https".into(),
            download: false,
        }
        .sanitized()?;
        request.alias = info.alias;
        Ok(Some(request))
    }
}

/// A device that scanned our pairing QR. Its certificate is what becomes trusted,
/// so a connection without one has nothing to pair.
async fn pair(
    State(app): State<Arc<App>>,
    Extension(conn): Extension<ConnInfo>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let Some(id) = conn.cert_fingerprint else {
        return error(403, "A client certificate is required");
    };
    if !is_localsend_id(&id) || id.eq_ignore_ascii_case(&app.identity.fingerprint) {
        return error(400, "Invalid device");
    }
    let request = match PairRequest::parse(&body) {
        Ok(request) => request,
        Err(message) => return error(400, message),
    };
    let token = query.get("token").map_or("", String::as_str);
    if !app.redeem_pair_token(token) {
        return error(403, "This QR code has expired");
    }
    if app.set_trusted(&id, true).is_err() {
        return error(500, "Could not save the pairing");
    }
    let peer = app.registry.get(&id);
    let request = request.unwrap_or_else(|| PairRequest {
        alias: peer
            .as_ref()
            .map_or_else(|| "Paired device".into(), |p| p.info.alias.clone()),
        port: peer
            .as_ref()
            .map_or(crate::config::DEFAULT_PORT, |p| p.info.port),
        addresses: Vec::new(),
    });
    let source = match net::canonical(conn.ip) {
        IpAddr::V4(ip) => Some(ip),
        _ => None,
    };
    // Trust is saved; the addresses are only hints for reaching it later.
    let _ = app.update_settings(|s| {
        s.remember_device(
            &id,
            &request.alias,
            request.port,
            source.into_iter().chain(request.addresses),
        );
    });
    app.bus.emit("paired", json!({ "alias": request.alias }));
    Json(json!({})).into_response()
}

fn check_pin(app: &App, conn: &ConnInfo, given: Option<&String>) -> Option<Response> {
    let pin = app.settings().pin.clone();
    if pin.is_empty() {
        return None;
    }
    if !app.pin_failures.allows(conn.ip) {
        return Some(error(429, "Too many requests"));
    }
    match given {
        None => Some(error(401, "PIN required")),
        Some(given) if same_secret(given, &pin) => None,
        Some(_) => {
            app.pin_failures.record(conn.ip);
            Some(error(401, "Invalid PIN"))
        }
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Compares secrets without revealing their length or where they differ.
pub fn same_secret(a: &str, b: &str) -> bool {
    use sha2::{Digest, Sha256};
    constant_time_eq(&Sha256::digest(a), &Sha256::digest(b))
}

/// Marks a waiting transfer as cancelled if the sender hangs up while we wait
/// for the user (the request future is then dropped).
pub struct WaitGuard {
    pub app: Arc<App>,
    pub id: String,
    pub armed: bool,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if self.armed {
            self.app.finish(
                &self.id,
                Status::Cancelled,
                Some("The sender cancelled".into()),
            );
        }
    }
}

async fn prepare_upload(
    State(app): State<Arc<App>>,
    Extension(conn): Extension<ConnInfo>,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Some(response) = check_pin(&app, &conn, query.get("pin")) {
        return response;
    }
    let Ok(request) = serde_json::from_slice::<PrepareUploadRequest>(&body) else {
        return error(400, "Invalid body");
    };
    let Some((info, verified)) = parse_info(&app, &conn, request.info) else {
        return error(400, "Invalid sender");
    };
    if request.files.len() > MAX_FILES {
        return error(413, "Too many files");
    }
    let files: Result<Vec<FileDto>, _> = request
        .files
        .into_values()
        .map(FileDto::sanitized)
        .collect();
    let Ok(files) = files else {
        return error(400, "Invalid files");
    };
    if files.is_empty() {
        return error(400, "No files provided");
    }
    let mut ids = std::collections::HashSet::new();
    if !files.iter().all(|f| ids.insert(f.id.as_str())) {
        return error(400, "Duplicate file ids");
    }
    if files
        .iter()
        .try_fold(0u64, |total, f| total.checked_add(f.size))
        .is_none()
    {
        return error(400, "Invalid file sizes");
    }
    // Every request reaches the user (a prompt or a message): not too many.
    if !app.requests.hit(conn.ip) {
        return error(429, "Too many requests");
    }

    let peer = app.upsert_peer(info, conn.ip, verified);
    let message = (files.len() == 1 && files[0].is_text_message())
        .then(|| files[0].preview.clone())
        .flatten();
    let states = files
        .into_iter()
        .map(|dto| FileState::new(dto, None))
        .collect();
    let mut transfer = Transfer::new(
        Direction::In,
        peer.id(),
        app.registry.peer_json(&peer),
        states,
    );
    transfer.peer_ip = Some(conn.ip);
    // Only a proven id counts as one of your devices.
    transfer.peer_verified = verified;

    if let Some(text) = message {
        // Text is shown, not saved, so it needs no confirmation.
        transfer.message = Some(text);
        transfer.started = Some(now());
        for f in transfer.files.iter_mut() {
            f.status = FileStatus::Done;
            f.done = f.size;
        }
        let id = app.transfers.lock().unwrap().add(transfer);
        app.finish(&id, Status::Done, None);
        app.on_text_received(&id);
        return StatusCode::NO_CONTENT.into_response();
    }

    transfer.session_id = Some(uuid::Uuid::new_v4().to_string());
    let total = transfer.total();
    let id = {
        let mut transfers = app.transfers.lock().unwrap();
        if transfers.active_incoming().is_some() {
            return error(409, "Blocked by another session");
        }
        transfers.add(transfer)
    };

    let mut guard = WaitGuard {
        app: app.clone(),
        id: id.clone(),
        armed: true,
    };
    let accepted = app.ask_accept(&id).await;
    guard.armed = false;

    if app
        .transfers
        .lock()
        .unwrap()
        .get(&id)
        .is_none_or(|t| t.status.is_final())
    {
        return error(403, "Cancelled");
    }
    if !accepted {
        app.finish(&id, Status::Declined, None);
        return error(403, "Rejected");
    }
    let download_dir = app.settings().download_dir.clone();
    let _ = std::fs::create_dir_all(&download_dir);
    if storage::free_space(&download_dir).is_some_and(|free| free < total) {
        app.finish(&id, Status::Failed, Some("Not enough disk space".into()));
        return error(500, "Not enough disk space on the receiver");
    }

    let mut transfers = app.transfers.lock().unwrap();
    let Some(transfer) = transfers.get_mut(&id) else {
        return error(500, "Session lost");
    };
    let mut tokens = HashMap::new();
    for f in transfer.files.iter_mut() {
        let token = Alphanumeric.sample_string(&mut rand::thread_rng(), 24);
        f.token = Some(token.clone());
        tokens.insert(f.id.clone(), token);
    }
    transfer.status = Status::Transferring;
    transfer.started = Some(now());
    let session_id = transfer.session_id.clone().unwrap_or_default();
    transfers.emit(&id, true);
    Json(json!({ "sessionId": session_id, "files": tokens })).into_response()
}

async fn upload(
    State(app): State<Arc<App>>,
    Extension(conn): Extension<ConnInfo>,
    Query(query): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Response {
    let (Some(session), Some(file_id), Some(token)) = (
        query.get("sessionId"),
        query.get("fileId"),
        query.get("token"),
    ) else {
        return error(400, "Missing parameters");
    };
    let (transfer_id, dto) = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.active_incoming() else {
            return error(403, "Invalid session");
        };
        if t.session_id.as_deref() != Some(session.as_str()) {
            return error(403, "Invalid session");
        }
        if t.peer_ip != Some(conn.ip) {
            return error(403, "Invalid IP address");
        }
        let Some(f) = t.file(file_id) else {
            return error(403, "Invalid file");
        };
        let valid = f
            .token
            .as_deref()
            .is_some_and(|expected| constant_time_eq(expected.as_bytes(), token.as_bytes()));
        if !valid {
            return error(403, "Invalid token");
        }
        if f.status == FileStatus::Done {
            return error(409, "Already received");
        }
        (t.id.clone(), f.dto.clone().unwrap_or_default())
    };
    let declared = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > dto.size) {
        return error(413, "More data than announced");
    }
    match storage::save_stream(&app, &transfer_id, &dto, body.into_data_stream()).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(SaveError::ChecksumMismatch) => error(422, "Checksum mismatch"),
        Err(SaveError::Aborted) => error(409, "Session cancelled"),
        Err(SaveError::Io(err)) => {
            tracing::warn!("receiving {} failed: {err}", dto.file_name);
            error(500, "Could not save the file")
        }
    }
}

async fn cancel(
    State(app): State<Arc<App>>,
    Extension(conn): Extension<ConnInfo>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(session) = query.get("sessionId") else {
        return error(400, "Missing parameters");
    };
    let incoming = {
        let transfers = app.transfers.lock().unwrap();
        transfers
            .active_incoming()
            .filter(|t| {
                t.session_id.as_deref() == Some(session.as_str()) && t.peer_ip == Some(conn.ip)
            })
            .map(|t| t.id.clone())
    };
    if let Some(id) = incoming {
        app.finish(&id, Status::Cancelled, Some("The sender cancelled".into()));
        return StatusCode::OK.into_response();
    }
    // The receiver of one of *our* transfers cancelled it.
    let outgoing: Vec<String> = {
        let mut transfers = app.transfers.lock().unwrap();
        let ids: Vec<String> = transfers
            .iter()
            .filter(|t| {
                t.direction == Direction::Out
                    && !t.status.is_final()
                    && t.remote_session_id.as_deref() == Some(session.as_str())
                    && t.peer_ip == Some(conn.ip)
            })
            .map(|t| t.id.clone())
            .collect();
        for id in &ids {
            if let Some(t) = transfers.get_mut(id) {
                t.error = Some("The receiver cancelled".into());
            }
        }
        ids
    };
    for id in outgoing {
        app.cancel(&id);
    }
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_body_is_optional_but_validated_when_present() {
        assert!(PairRequest::parse(b"").unwrap().is_none());
        assert!(PairRequest::parse(b" \n").unwrap().is_none());
        let request = PairRequest::parse(
            br#"{"alias":"  Phone\u202e  ","port":53317,"addresses":["100.64.1.2","192.168.1.2"]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(request.alias, "Phone");
        assert_eq!(request.addresses[0], Ipv4Addr::new(100, 64, 1, 2));
        for address in [
            "8.8.8.8",
            "127.0.0.1",
            "169.254.1.2",
            "100.128.0.1",
            "localhost",
            "::ffff:100.64.1.2",
            "100.064.1.2",
        ] {
            let body = json!({"alias":"Phone", "port":53317, "addresses":[address]}).to_string();
            assert!(PairRequest::parse(body.as_bytes()).is_err(), "{address}");
        }
        for port in [0, 22, 1023, 65536] {
            let body = json!({"alias":"Phone", "port":port, "addresses":[]}).to_string();
            assert!(PairRequest::parse(body.as_bytes()).is_err());
        }
        for body in [
            "null",
            "[]",
            "{}",
            "invalid",
            r#"{"alias":" ","port":53317,"addresses":[]}"#,
        ] {
            assert!(PairRequest::parse(body.as_bytes()).is_err());
        }
        let body =
            json!({"alias":"Phone", "port":1024, "addresses":vec!["10.0.0.1"; 8]}).to_string();
        assert!(PairRequest::parse(body.as_bytes()).is_ok());
        let body =
            json!({"alias":"Phone", "port":53317, "addresses":vec!["10.0.0.1"; 9]}).to_string();
        assert!(PairRequest::parse(body.as_bytes()).is_err());
    }
}
