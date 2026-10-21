//! Web companion: a phone without the app scans the QR code shown on the
//! desktop and shares through its browser. The QR code carries a secret key,
//! so only someone who could see this screen can connect.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::io::ReaderStream;

use crate::app::App;
use crate::assets;
use crate::net;
use crate::peers::{Peer, PeerKind};
use crate::protocol::{safe_relative_path, DeviceInfo, FileDto};
use crate::receiver::constant_time_eq;
use crate::sender::SendItem;
use crate::storage::{self, SaveError};
use crate::transfers::{now, Direction, FileState, FileStatus, Status, Transfer};

type Params = Query<HashMap<String, String>>;

#[derive(Default)]
pub struct Hub {
    clients: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
}

impl Hub {
    fn push(&self, peer_id: &str, message: Value) -> bool {
        match self.clients.lock().unwrap().get(peer_id) {
            Some(tx) => tx.send(message.to_string()).is_ok(),
            None => false,
        }
    }

    /// Offers files or text from the desktop to a connected browser.
    pub fn offer(&self, app: &Arc<App>, peer: &Peer, items: Vec<SendItem>, text: Option<String>) -> Result<String, String> {
        if !self.clients.lock().unwrap().contains_key(peer.id()) {
            return Err("That browser is no longer connected".into());
        }
        let mut files = Vec::new();
        for item in &items {
            let dto = FileDto::for_path(&item.path, &item.name).map_err(|e| format!("{}: {e}", item.path.display()))?;
            files.push(FileState::new(dto, Some(item.path.to_string_lossy().into_owned())));
        }
        if let Some(text) = &text {
            files.push(FileState::new(FileDto::for_text(text), None));
        }
        if files.is_empty() {
            return Err("Nothing to send".into());
        }
        let mut transfer = Transfer::new(Direction::Out, peer.id(), app.registry.peer_json(peer), files);
        transfer.peer_ip = Some(peer.ip);
        transfer.message = text;
        transfer.cleanup = items.iter().filter(|i| i.temporary).map(|i| i.path.clone()).collect();
        let offer = json!({
            "type": "offer",
            "id": transfer.id,
            "from": app.me().alias,
            "message": transfer.message,
            "files": transfer.files.iter().map(|f| json!({ "id": f.id, "name": f.name, "size": f.size, "type": f.file_type })).collect::<Vec<_>>(),
        });
        let id = app.transfers.lock().unwrap().add(transfer);
        if !self.push(peer.id(), offer) {
            app.finish(&id, Status::Failed, Some("That browser is no longer connected".into()));
            return Err("That browser is no longer connected".into());
        }
        Ok(id)
    }

    /// Drops every browser connection (after the pairing key changed).
    pub fn disconnect_all(&self) {
        self.clients.lock().unwrap().clear();
    }

    pub fn cancel_offer(&self, app: &App, transfer_id: &str) {
        let peer_id = app.transfers.lock().unwrap().get(transfer_id).map(|t| t.peer_id.clone());
        if let Some(peer_id) = peer_id {
            self.push(&peer_id, json!({ "type": "cancel", "id": transfer_id }));
        }
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/assets/{*path}", get(asset))
        .route("/hello", get(hello))
        .route("/ws", get(websocket))
        .route("/send", post(announce_files))
        .route("/upload", post(upload))
        .route("/text", post(text))
        .route("/offer/{id}/{action}", post(answer_offer))
        .route("/download/{id}/{file}", get(download))
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(app)
}

pub async fn serve(app: Arc<App>, listener: tokio::net::TcpListener) {
    let service = router(app).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(err) = axum::serve(listener, service).await {
        tracing::warn!("web companion stopped: {err}");
    }
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "message": "Scan the QR code in AirBridge on your computer" }))).into_response()
}

fn authorized(app: &App, params: &HashMap<String, String>) -> bool {
    let settings = app.settings();
    settings.web_companion
        && params.get("k").is_some_and(|k| constant_time_eq(k.as_bytes(), settings.web_key.as_bytes()))
}

/// "web-" + the random id the page keeps in localStorage.
fn client_id(params: &HashMap<String, String>) -> Option<String> {
    let raw = params.get("id")?;
    let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    (clean.len() >= 8).then(|| format!("web-{clean}"))
}

async fn page() -> Response {
    assets::response("companion.html")
}

async fn asset(UrlPath(path): UrlPath<String>) -> Response {
    assets::response(&path)
}

/// Lets the page check its pairing key before connecting.
async fn hello(State(app): State<Arc<App>>, Query(params): Params) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let me = app.me();
    Json(json!({ "alias": me.alias, "deviceModel": me.device_model })).into_response()
}

async fn websocket(
    ws: WebSocketUpgrade,
    State(app): State<Arc<App>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Params,
) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let Some(peer_id) = client_id(&params) else { return (StatusCode::BAD_REQUEST, "missing id").into_response() };
    let name: String = params.get("name").map(|n| n.trim().chars().take(48).collect()).unwrap_or_default();
    let model: Option<String> = params.get("model").map(|m| m.chars().take(64).collect()).filter(|m: &String| !m.is_empty());
    let ip = net::canonical(addr.ip());
    ws.on_upgrade(move |socket| client_session(app, socket, ip, peer_id, name, model))
}

async fn client_session(app: Arc<App>, socket: WebSocket, ip: IpAddr, peer_id: String, name: String, model: Option<String>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    app.companion.clients.lock().unwrap().insert(peer_id.clone(), tx.clone());
    let info = DeviceInfo {
        alias: if name.is_empty() { "Phone (browser)".into() } else { name },
        version: "web".into(),
        device_model: model,
        device_type: Some("web".into()),
        fingerprint: peer_id.clone(),
        port: 0,
        protocol: "http".into(),
        download: false,
    };
    app.registry.upsert(info, ip, PeerKind::Web);

    let (mut sink, mut stream) = socket.split();
    let hello = json!({ "type": "hello", "alias": app.me().alias, "deviceModel": app.me().device_model });
    let _ = sink.send(Message::Text(hello.to_string().into())).await;
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            outgoing = rx.recv() => match outgoing {
                Some(text) => if sink.send(Message::Text(text.into())).await.is_err() { break },
                None => break,
            },
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() { break }
            }
        }
    }

    // Only forget the client if a newer connection did not replace it.
    let current = {
        let mut clients = app.companion.clients.lock().unwrap();
        // A cleared entry (pairing reset) also counts as ours.
        let ours = clients.get(&peer_id).is_none_or(|c| c.same_channel(&tx));
        if ours {
            clients.remove(&peer_id);
        }
        ours
    };
    if current {
        app.registry.remove(&peer_id);
        let waiting: Vec<String> = app
            .transfers
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.peer_id == peer_id && t.status == Status::Waiting)
            .map(|t| t.id.clone())
            .collect();
        for id in waiting {
            app.finish(&id, Status::Failed, Some("The browser disconnected".into()));
        }
    }
}

#[derive(Deserialize)]
struct Announced {
    files: Vec<AnnouncedFile>,
}

#[derive(Deserialize)]
struct AnnouncedFile {
    name: String,
    size: u64,
    #[serde(default, rename = "type")]
    file_type: String,
}

/// The phone announces the files it is about to upload.
async fn announce_files(
    State(app): State<Arc<App>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Params,
    Json(body): Json<Announced>,
) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let Some(peer_id) = client_id(&params) else { return (StatusCode::BAD_REQUEST, "missing id").into_response() };
    if body.files.is_empty() || body.files.len() > 10_000 {
        return (StatusCode::BAD_REQUEST, "no files").into_response();
    }
    let ip = net::canonical(addr.ip());
    let peer_json = app
        .registry
        .get(&peer_id)
        .map(|p| app.registry.peer_json(&p))
        .unwrap_or_else(|| json!({ "id": peer_id, "alias": "Phone (browser)", "deviceType": "web", "kind": "web" }));
    let files: Vec<FileState> = body
        .files
        .into_iter()
        .map(|f| {
            let dto = FileDto {
                id: uuid::Uuid::new_v4().to_string(),
                file_name: f.name.chars().take(1024).collect(),
                size: f.size,
                file_type: if f.file_type.is_empty() { "application/octet-stream".into() } else { f.file_type },
                ..Default::default()
            };
            FileState::new(dto, None)
        })
        .collect();
    let mut transfer = Transfer::new(Direction::In, &peer_id, peer_json, files);
    transfer.peer_ip = Some(ip);
    // Whoever holds the QR key is you: no confirmation needed.
    transfer.status = Status::Transferring;
    transfer.started = Some(now());
    let total = transfer.total();
    let download_dir = app.settings().download_dir.clone();
    let _ = std::fs::create_dir_all(&download_dir);
    if storage::free_space(&download_dir).is_some_and(|free| free < total) {
        return (StatusCode::INSUFFICIENT_STORAGE, Json(json!({ "message": "Not enough space on the computer" }))).into_response();
    }
    let ids: Vec<Value> = transfer.files.iter().map(|f| json!({ "id": f.id, "name": f.name })).collect();
    let id = app.transfers.lock().unwrap().add(transfer);
    Json(json!({ "transfer": id, "files": ids })).into_response()
}

async fn upload(State(app): State<Arc<App>>, Query(params): Params, body: Body) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let (Some(peer_id), Some(transfer_id), Some(file_id)) = (client_id(&params), params.get("transfer"), params.get("file"))
    else {
        return (StatusCode::BAD_REQUEST, "missing parameters").into_response();
    };
    let dto = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(transfer_id) else { return forbidden() };
        if t.direction != Direction::In || t.peer_id != peer_id || t.status.is_final() {
            return (StatusCode::CONFLICT, "transfer is over").into_response();
        }
        match t.file(file_id) {
            Some(f) if f.status != FileStatus::Done => f.dto.clone().unwrap_or_default(),
            _ => return (StatusCode::CONFLICT, "unknown file").into_response(),
        }
    };
    match storage::save_stream(&app, transfer_id, &dto, body.into_data_stream()).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(SaveError::Aborted) => (StatusCode::CONFLICT, "cancelled").into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn text(
    State(app): State<Arc<App>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(params): Params,
    body: String,
) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let Some(peer_id) = client_id(&params) else { return (StatusCode::BAD_REQUEST, "missing id").into_response() };
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty").into_response();
    }
    let peer_json = app
        .registry
        .get(&peer_id)
        .map(|p| app.registry.peer_json(&p))
        .unwrap_or_else(|| json!({ "id": peer_id, "alias": "Phone (browser)", "deviceType": "web", "kind": "web" }));
    let mut state = FileState::new(FileDto::for_text(&body), None);
    state.status = FileStatus::Done;
    state.done = state.size;
    let mut transfer = Transfer::new(Direction::In, &peer_id, peer_json, vec![state]);
    transfer.peer_ip = Some(net::canonical(addr.ip()));
    transfer.message = Some(body);
    transfer.started = Some(now());
    let id = app.transfers.lock().unwrap().add(transfer);
    app.finish(&id, Status::Done, None);
    app.on_text_received(&id);
    Json(json!({ "ok": true })).into_response()
}

async fn answer_offer(
    State(app): State<Arc<App>>,
    UrlPath((id, action)): UrlPath<(String, String)>,
    Query(params): Params,
) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let Some(peer_id) = client_id(&params) else { return (StatusCode::BAD_REQUEST, "missing id").into_response() };
    let is_text = {
        let mut transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get_mut(&id) else { return (StatusCode::NOT_FOUND, "unknown").into_response() };
        if t.peer_id != peer_id || t.direction != Direction::Out {
            return forbidden();
        }
        if t.status != Status::Waiting && action != "decline" {
            return (StatusCode::CONFLICT, "not waiting").into_response();
        }
        if action == "accept" {
            t.status = Status::Transferring;
            t.started = Some(now());
        }
        let is_text = t.message.is_some() && t.files.iter().all(|f| f.path.is_none());
        transfers.emit(&id, true);
        is_text
    };
    match action.as_str() {
        "accept" if is_text => {
            {
                let mut transfers = app.transfers.lock().unwrap();
                if let Some(t) = transfers.get_mut(&id) {
                    for f in t.files.iter_mut() {
                        f.status = FileStatus::Done;
                        f.done = f.size;
                    }
                }
            }
            app.finish(&id, Status::Done, None);
        }
        "accept" => {}
        "decline" => {
            app.finish(&id, Status::Declined, Some("Declined".into()));
        }
        _ => return (StatusCode::BAD_REQUEST, "unknown action").into_response(),
    }
    Json(json!({ "ok": true })).into_response()
}

fn content_disposition(name: &str) -> String {
    let base = safe_relative_path(name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ascii: String = base.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = base
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

async fn download(
    State(app): State<Arc<App>>,
    UrlPath((id, file_id)): UrlPath<(String, String)>,
    Query(params): Params,
) -> Response {
    if !authorized(&app, &params) {
        return forbidden();
    }
    let (path, name, size, mime) = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(&id) else { return (StatusCode::NOT_FOUND, "unknown").into_response() };
        if t.direction != Direction::Out || !matches!(t.status, Status::Transferring | Status::Done) {
            return (StatusCode::CONFLICT, "not accepted").into_response();
        }
        let Some(f) = t.file(&file_id) else { return (StatusCode::NOT_FOUND, "unknown file").into_response() };
        let Some(path) = f.path.clone() else { return (StatusCode::NOT_FOUND, "no data").into_response() };
        (path, f.name.clone(), f.size, f.file_type.clone())
    };
    if size == 0 {
        let complete = {
            let mut transfers = app.transfers.lock().unwrap();
            if let Some(f) = transfers.get_mut(&id).and_then(|t| t.file_mut(&file_id)) {
                f.status = FileStatus::Done;
            }
            transfers.get(&id).is_some_and(|t| t.files.iter().all(|f| f.status == FileStatus::Done || f.path.is_none()))
        };
        if complete {
            app.finish(&id, Status::Done, None);
        }
    }
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(err) => return (StatusCode::GONE, err.to_string()).into_response(),
    };
    let mut sent = 0u64;
    let (progress_app, progress_id, progress_file) = (app.clone(), id.clone(), file_id.clone());
    let stream = ReaderStream::with_capacity(file, 256 * 1024).inspect(move |chunk| {
        let Ok(chunk) = chunk else { return };
        sent += chunk.len() as u64;
        let complete = {
            let mut transfers = progress_app.transfers.lock().unwrap();
            if let Some(f) = transfers.get_mut(&progress_id).and_then(|t| t.file_mut(&progress_file)) {
                f.done = sent.min(f.size);
                if sent >= f.size {
                    f.status = FileStatus::Done;
                } else {
                    f.status = FileStatus::Transferring;
                }
            }
            transfers.emit(&progress_id, sent >= size);
            sent >= size
                && transfers
                    .get(&progress_id)
                    .is_some_and(|t| t.files.iter().all(|f| f.status == FileStatus::Done || f.path.is_none()))
        };
        if complete {
            progress_app.finish(&progress_id, Status::Done, None);
        }
    });
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, size)
        .header(header::CONTENT_DISPOSITION, content_disposition(&name))
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
