//! Local API behind the desktop window. Bound to 127.0.0.1 and protected by a
//! random token (cookie for the window, header for the CLI).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast::error::RecvError;

use crate::app::App;
use crate::assets;
use crate::config;
use crate::desktop;
use crate::receiver::constant_time_eq;
use crate::sender::{expand_paths, SendItem};

const COOKIE: &str = "airbridge_token";

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "message": message.into() })),
    )
        .into_response()
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/ping", get(ping))
        .route("/state", get(state))
        .route("/events", get(events))
        .route("/send", post(send))
        .route("/send-text", post(send_text))
        .route("/send-clipboard", post(send_clipboard))
        .route("/send-url", post(send_url))
        .route("/wake", post(wake))
        .route("/stage", post(stage))
        .route("/stage/{id}", delete(unstage))
        .route("/stage-paths", post(stage_paths))
        .route("/pick", post(pick))
        .route("/transfers/{id}", get(transfer))
        .route("/transfers/{id}/{action}", post(transfer_action))
        .route("/transfers/{id}/files/{file}/preview", get(preview))
        .route("/history", delete(clear_history))
        .route("/refresh", post(refresh))
        .route("/scan", post(scan))
        .route("/settings", post(update_settings))
        .route("/reset-pairing", post(reset_pairing))
        .route("/peers/{id}/trust", post(trust))
        .route("/qr.svg", get(qr))
        .route("/copy", post(copy))
        .route("/open-downloads", post(open_downloads))
        .route("/show", post(show))
        .route("/quit", post(quit))
        .layer(middleware::from_fn_with_state(app.clone(), auth));
    Router::new()
        .route("/", get(index))
        .route(
            "/assets/{*path}",
            get(|UrlPath(path): UrlPath<String>| async move { assets::response(&path) }),
        )
        .nest("/api", api)
        .with_state(app)
}

fn token(app: &App) -> &str {
    app.ui.get().map(|ui| ui.token.as_str()).unwrap_or("")
}

fn has_token(app: &App, headers: &HeaderMap) -> bool {
    let expected = token(app).as_bytes();
    if expected.is_empty() {
        return false;
    }
    if let Some(value) = headers
        .get("x-airbridge-token")
        .and_then(|v| v.to_str().ok())
    {
        return constant_time_eq(value.as_bytes(), expected);
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| {
            pair.trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_string)
        })
        .any(|value| constant_time_eq(value.as_bytes(), expected))
}

/// Blocks DNS-rebinding: the page must be addressed as localhost.
fn host_ok(app: &App, headers: &HeaderMap) -> bool {
    let port = app.ui.get().map(|ui| ui.port).unwrap_or(0);
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
    .iter()
    .any(|h| h == host)
}

async fn auth(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    if !host_ok(&app, request.headers()) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    if !has_token(&app, request.headers()) {
        return (StatusCode::UNAUTHORIZED, "missing token").into_response();
    }
    next.run(request).await
}

#[derive(Deserialize)]
struct IndexQuery {
    token: Option<String>,
}

async fn index(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(query): Query<IndexQuery>,
) -> Response {
    if !host_ok(&app, &headers) {
        return (StatusCode::FORBIDDEN, "bad host").into_response();
    }
    if let Some(given) = query.token {
        if constant_time_eq(given.as_bytes(), token(&app).as_bytes()) {
            let cookie = format!("{COOKIE}={given}; Path=/; HttpOnly; SameSite=Strict");
            return ([(header::SET_COOKIE, cookie)], Redirect::to("/")).into_response();
        }
    }
    if has_token(&app, &headers) {
        return assets::response("index.html");
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "<!doctype html><meta charset=utf-8><title>AirBridge</title>\
         <body style=\"font:16px system-ui;padding:3em;background:#0b0d17;color:#eee\">\
         Open AirBridge from your applications menu (or run <code>airbridge</code>).",
    )
        .into_response()
}

async fn ping() -> Json<Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() }))
}

async fn state(State(app): State<Arc<App>>) -> Json<Value> {
    Json(app.state_json())
}

async fn events(ws: WebSocketUpgrade, State(app): State<Arc<App>>) -> Response {
    ws.on_upgrade(move |socket| event_stream(app, socket))
}

async fn event_stream(app: Arc<App>, socket: WebSocket) {
    app.ui_clients.fetch_add(1, Ordering::Relaxed);
    // Opening the window means "I want to share": wake nearby phones.
    app.wake_phones();
    let mut events = app.bus.subscribe();
    let (mut sink, mut incoming) = socket.split();
    let state = json!({ "event": "state", "data": app.state_json() }).to_string();
    if sink.send(Message::Text(state.into())).await.is_ok() {
        loop {
            tokio::select! {
                event = events.recv() => {
                    let text = match event {
                        Ok(event) => event.to_json_string(),
                        Err(RecvError::Lagged(_)) => json!({ "event": "state", "data": app.state_json() }).to_string(),
                        Err(RecvError::Closed) => break,
                    };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                message = incoming.next() => match message {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
                _ = app.shutdown.cancelled() => break,
            }
        }
    }
    app.ui_clients.fetch_sub(1, Ordering::Relaxed);
}

// ------------------------------------------------------------------ sending

#[derive(Deserialize)]
struct SendRequest {
    peer: String,
    #[serde(default)]
    paths: Vec<PathBuf>,
    #[serde(default)]
    staged: Vec<String>,
    #[serde(default)]
    pin: Option<String>,
}

async fn send(State(app): State<Arc<App>>, Json(request): Json<SendRequest>) -> Response {
    let mut items = expand_paths(&request.paths);
    {
        let mut staged = app.staged.lock().unwrap();
        for id in &request.staged {
            if let Some(item) = staged.remove(id) {
                items.push(item);
            }
        }
    }
    if items.is_empty() {
        return bad_request("Nothing to send");
    }
    match app.send_files(&request.peer, items, request.pin) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(message) => bad_request(message),
    }
}

#[derive(Deserialize)]
struct TextRequest {
    peer: String,
    text: String,
    #[serde(default)]
    pin: Option<String>,
}

async fn send_text(State(app): State<Arc<App>>, Json(request): Json<TextRequest>) -> Response {
    match app.send_text(&request.peer, request.text, request.pin) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(message) => bad_request(message),
    }
}

#[derive(Deserialize)]
struct PeerRequest {
    peer: String,
}

async fn send_clipboard(State(app): State<Arc<App>>, Json(request): Json<PeerRequest>) -> Response {
    let Some(text) = desktop::read_clipboard().await.filter(|t| !t.is_empty()) else {
        return bad_request("The clipboard is empty (or wl-paste/xclip is not installed)");
    };
    match app.send_text(&request.peer, text, None) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(message) => bad_request(message),
    }
}

#[derive(Deserialize)]
struct UrlRequest {
    peer: String,
    url: String,
    /// File name suggested by the drag source.
    #[serde(default)]
    name: Option<String>,
}

/// Something dragged out of a web page: images and files are downloaded and
/// sent as files, web pages are sent as links.
async fn send_url(State(app): State<Arc<App>>, Json(request): Json<UrlRequest>) -> Response {
    let url = request.url.trim().to_string();
    match crate::fetch::fetch(&url, request.name.as_deref()).await {
        Ok(crate::fetch::Fetched::File(item)) => {
            match app.send_files(&request.peer, vec![item.clone()], None) {
                Ok(id) => Json(json!({ "id": id, "kind": "file" })).into_response(),
                Err(message) => {
                    let _ = std::fs::remove_dir_all(item.path.parent().unwrap_or(&item.path));
                    bad_request(message)
                }
            }
        }
        Ok(crate::fetch::Fetched::Page) => match app.send_text(&request.peer, url, None) {
            Ok(id) => Json(json!({ "id": id, "kind": "link" })).into_response(),
            Err(message) => bad_request(message),
        },
        Err(err) => bad_request(format!("Could not download it: {err}")),
    }
}

/// Someone is about to share (e.g. a drag reached the screen edge).
async fn wake(State(app): State<Arc<App>>) -> StatusCode {
    app.wake_phones();
    StatusCode::NO_CONTENT
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Receives a file dropped into the window (browsers do not reveal paths) and
/// keeps it in the cache until it is sent.
async fn stage(State(app): State<Arc<App>>, headers: HeaderMap, body: Body) -> Response {
    app.wake_phones();
    let name = headers
        .get("x-file-name")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "file".into());
    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = config::cache_dir().join("staging").join(&id);
    let base = crate::protocol::safe_relative_path(&name)
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "file".into());
    let path = dir.join(base);
    let result: std::io::Result<u64> = async {
        tokio::fs::create_dir_all(&dir).await?;
        let mut file = tokio::fs::File::create(&path).await?;
        let mut stream = body.into_data_stream();
        let mut size = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(std::io::Error::other)?;
            size += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(size)
    }
    .await;
    match result {
        Ok(size) => {
            let item = SendItem {
                path,
                name: name.clone(),
                temporary: true,
            };
            app.staged.lock().unwrap().insert(id.clone(), item);
            Json(json!({ "id": id, "name": name, "size": size })).into_response()
        }
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "message": err.to_string() })),
            )
                .into_response()
        }
    }
}

async fn unstage(State(app): State<Arc<App>>, UrlPath(id): UrlPath<String>) -> StatusCode {
    if let Some(item) = app.staged.lock().unwrap().remove(&id) {
        let _ = std::fs::remove_file(&item.path);
        if let Some(parent) = item.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    StatusCode::NO_CONTENT
}

fn describe_paths(paths: &[PathBuf]) -> Vec<Value> {
    paths
        .iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(path).ok()?;
            let size = if meta.is_dir() {
                expand_paths(std::slice::from_ref(path)).iter().filter_map(|i| std::fs::metadata(&i.path).ok()).map(|m| m.len()).sum()
            } else {
                meta.len()
            };
            Some(json!({
                "path": path,
                "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                "size": size,
                "isDir": meta.is_dir(),
                "type": mime_guess::from_path(path).first_or_octet_stream().essence_str(),
            }))
        })
        .collect()
}

#[derive(Deserialize)]
struct PathsRequest {
    paths: Vec<PathBuf>,
}

/// From the CLI / file manager: show the files in the window, ready to send.
async fn stage_paths(
    State(app): State<Arc<App>>,
    Json(request): Json<PathsRequest>,
) -> Json<Value> {
    let files = describe_paths(&request.paths);
    app.wake_phones();
    app.open_window();
    app.bus.emit("staged", json!({ "files": files }));
    Json(json!({ "files": files }))
}

#[derive(Deserialize)]
struct PickRequest {
    #[serde(default)]
    directory: bool,
}

async fn pick(Json(request): Json<PickRequest>) -> Json<Value> {
    let paths = desktop::pick(request.directory).await;
    Json(json!({ "files": describe_paths(&paths) }))
}

// ---------------------------------------------------------------- transfers

async fn transfer(State(app): State<Arc<App>>, UrlPath(id): UrlPath<String>) -> Response {
    match app.transfers.lock().unwrap().get(&id) {
        Some(t) => Json(t.to_json()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize, Default)]
struct ActionRequest {
    #[serde(default)]
    trust: bool,
    #[serde(default)]
    file: Option<String>,
}

async fn transfer_action(
    State(app): State<Arc<App>>,
    UrlPath((id, action)): UrlPath<(String, String)>,
    body: Option<Json<ActionRequest>>,
) -> Response {
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let (peer_id, message, path) = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(&id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let file = match &request.file {
            Some(fid) => t.file(fid),
            None => t.files.iter().find(|f| f.path.is_some()),
        };
        (
            t.peer_id.clone(),
            t.message.clone(),
            file.and_then(|f| f.path.clone()).map(PathBuf::from),
        )
    };
    let ok = match action.as_str() {
        "accept" => {
            if request.trust {
                app.set_trusted(&peer_id, true);
            }
            app.decide(&id, true)
        }
        "decline" => app.decide(&id, false),
        "cancel" => {
            app.cancel(&id);
            true
        }
        "open" => path.is_some_and(|p| desktop::open_path(&p)),
        "reveal" => match path {
            Some(p) => desktop::reveal_path(&p).await,
            None => desktop::open_path(&app.settings().download_dir),
        },
        "copy" => match message {
            Some(text) => desktop::copy_text(&text).await,
            None => false,
        },
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    Json(json!({ "ok": ok })).into_response()
}

/// Image preview for the activity list (only files that belong to a transfer).
async fn preview(
    State(app): State<Arc<App>>,
    UrlPath((id, file)): UrlPath<(String, String)>,
) -> Response {
    let path = {
        let transfers = app.transfers.lock().unwrap();
        transfers
            .get(&id)
            .and_then(|t| t.file(&file))
            .filter(|f| f.file_type.starts_with("image/") && f.size < 64 * 1024 * 1024)
            .and_then(|f| f.path.clone())
    };
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&path)
                .first_or_octet_stream()
                .essence_str()
                .to_string();
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "max-age=3600".into()),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn clear_history(State(app): State<Arc<App>>) -> StatusCode {
    app.transfers.lock().unwrap().clear_history();
    StatusCode::NO_CONTENT
}

// --------------------------------------------------------------- discovery

async fn refresh(State(app): State<Arc<App>>) -> StatusCode {
    app.wake_phones();
    if let Some(discovery) = app.discovery.get().cloned() {
        let app = app.clone();
        tokio::spawn(async move { discovery.announce_burst(&app).await });
    }
    StatusCode::NO_CONTENT
}

async fn scan(State(app): State<Arc<App>>) -> StatusCode {
    if let Some(discovery) = app.discovery.get().cloned() {
        let app = app.clone();
        tokio::spawn(async move { discovery.scan(&app).await });
    }
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct TrustRequest {
    trusted: bool,
}

async fn trust(
    State(app): State<Arc<App>>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<TrustRequest>,
) -> StatusCode {
    app.set_trusted(&id, request.trusted);
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------- settings

async fn update_settings(State(app): State<Arc<App>>, Json(changes): Json<Value>) -> Json<Value> {
    let changed = app.update_settings(|s| s.apply(&changes));
    if changed.contains(&"alias") {
        if let Some(discovery) = app.discovery.get().cloned() {
            let app = app.clone();
            tokio::spawn(async move { discovery.announce_burst(&app).await });
        }
        let service = app.quick_share.lock().unwrap().clone();
        if let Some(service) = service {
            if let Err(err) = service.advertise(&app.settings().alias) {
                tracing::warn!("Could not update the Quick Share name: {err}");
            }
        }
    }
    if changed.contains(&"quickShare") {
        if app.settings().quick_share {
            crate::start_quick_share(&app).await;
        } else {
            crate::stop_quick_share(&app);
        }
    }
    if changed.contains(&"notifications") || changed.contains(&"webCompanion") {
        app.bus
            .emit("companion", json!({ "url": app.companion_url() }));
    }
    Json(app.settings().public_json())
}

async fn reset_pairing(State(app): State<Arc<App>>) -> Json<Value> {
    app.update_settings(|s| s.regenerate_web_key());
    app.companion.disconnect_all();
    let url = app.companion_url();
    app.bus.emit("companion", json!({ "url": url }));
    Json(json!({ "url": url }))
}

async fn qr(State(app): State<Arc<App>>) -> Response {
    let Some(url) = app.companion_url() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(220, 220)
                .quiet_zone(false)
                .dark_color(qrcode::render::svg::Color("#0b0d17"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build();
            (
                [
                    (header::CONTENT_TYPE, "image/svg+xml"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                svg,
            )
                .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct CopyRequest {
    text: String,
}

async fn copy(Json(request): Json<CopyRequest>) -> Json<Value> {
    Json(json!({ "ok": desktop::copy_text(&request.text).await }))
}

async fn open_downloads(State(app): State<Arc<App>>) -> StatusCode {
    let dir = app.settings().download_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    desktop::open_path(&dir);
    StatusCode::NO_CONTENT
}

async fn show(State(app): State<Arc<App>>) -> StatusCode {
    app.open_window();
    StatusCode::NO_CONTENT
}

async fn quit(State(app): State<Arc<App>>) -> StatusCode {
    app.shutdown.cancel();
    StatusCode::NO_CONTENT
}
