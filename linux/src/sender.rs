//! Sending files and text to LocalSend peers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::TryStreamExt;
use hyper::Method;
use serde_json::json;
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::client::{self, Body, ByteStream, Options, PeerError};
use crate::peers::Peer;
use crate::protocol::{FileDto, PrepareUploadResponse, API};
use crate::thumbnail;
use crate::transfers::{now, Direction, FileState, FileStatus, Status, Transfer};

const CHUNK: usize = 256 * 1024;
const PARALLEL_UPLOADS: usize = 4;

#[derive(Debug, Clone)]
pub struct SendItem {
    pub path: PathBuf,
    /// Name announced to the receiver; may contain folders ("Holiday/img1.jpg").
    pub name: String,
    /// A staged copy that is deleted once the transfer ends.
    pub temporary: bool,
}

/// Files are sent as-is; folders are sent recursively with relative names.
pub fn expand_paths(paths: &[PathBuf]) -> Vec<SendItem> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<SendItem>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if kind.is_dir() {
                walk(&entry.path(), &name, out);
            } else if kind.is_file() {
                out.push(SendItem {
                    path: entry.path(),
                    name,
                    temporary: false,
                });
            }
        }
    }
    let mut items = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        if path.is_dir() {
            walk(path, &name, &mut items);
        } else if path.is_file() {
            items.push(SendItem {
                path: path.clone(),
                name,
                temporary: false,
            });
        }
    }
    items
}

enum Outcome {
    Done,
    Declined,
    Failed(String),
}

impl From<PeerError> for Outcome {
    fn from(err: PeerError) -> Self {
        Outcome::Failed(err.to_string())
    }
}

pub fn send_files(
    app: &Arc<App>,
    peer: Peer,
    items: Vec<SendItem>,
    pin: Option<String>,
) -> Result<String, String> {
    if items.is_empty() {
        return Err("Nothing to send".into());
    }
    let mut files = Vec::with_capacity(items.len());
    for item in &items {
        let dto = FileDto::for_path(&item.path, &item.name)
            .map_err(|e| format!("{}: {e}", item.path.display()))?;
        files.push(FileState::new(
            dto,
            Some(item.path.to_string_lossy().into_owned()),
        ));
    }
    let mut transfer = Transfer::new(
        Direction::Out,
        peer.id(),
        app.registry.peer_json(&peer),
        files,
    );
    transfer.peer_ip = Some(peer.ip);
    transfer.cleanup = items
        .iter()
        .filter(|i| i.temporary)
        .map(|i| i.path.clone())
        .collect();
    Ok(start(app, transfer, peer, pin))
}

pub fn send_text(app: &Arc<App>, peer: Peer, text: String, pin: Option<String>) -> String {
    let dto = FileDto::for_text(&text);
    let mut transfer = Transfer::new(
        Direction::Out,
        peer.id(),
        app.registry.peer_json(&peer),
        vec![FileState::new(dto, None)],
    );
    transfer.peer_ip = Some(peer.ip);
    transfer.message = Some(text);
    start(app, transfer, peer, pin)
}

fn start(app: &Arc<App>, transfer: Transfer, peer: Peer, pin: Option<String>) -> String {
    let id = app.transfers.lock().unwrap().add(transfer);
    let token = CancellationToken::new();
    app.register_cancel(&id, token.clone());
    let (app, transfer_id) = (app.clone(), id.clone());
    tokio::spawn(async move {
        let outcome = tokio::select! {
            outcome = run(&app, &transfer_id, &peer, pin) => Some(outcome),
            _ = token.cancelled() => None,
        };
        app.unregister_cancel(&transfer_id);
        let cancelled_session = {
            let mut transfers = app.transfers.lock().unwrap();
            match outcome {
                Some(Outcome::Done) => {
                    if let Some(t) = transfers.get_mut(&transfer_id) {
                        for f in t
                            .files
                            .iter_mut()
                            .filter(|f| f.status != FileStatus::Skipped)
                        {
                            f.status = FileStatus::Done;
                            f.done = f.size;
                        }
                        t.started.get_or_insert_with(now);
                    }
                    transfers.finish(&transfer_id, Status::Done, None);
                    None
                }
                Some(Outcome::Declined) => {
                    transfers.finish(&transfer_id, Status::Declined, Some("Declined".into()));
                    None
                }
                Some(Outcome::Failed(error)) => {
                    transfers.finish(&transfer_id, Status::Failed, Some(error));
                    None
                }
                None => {
                    let remote = transfers
                        .get(&transfer_id)
                        .and_then(|t| t.remote_session_id.clone());
                    let reason = transfers.get(&transfer_id).and_then(|t| t.error.clone());
                    transfers.finish(
                        &transfer_id,
                        Status::Cancelled,
                        reason.or(Some("Cancelled".into())),
                    );
                    remote
                }
            }
        };
        if let Some(session) = cancelled_session {
            cancel_remote(&app, &peer, &session).await;
        }
    });
    id
}

/// Up to three photo previews, so the receiver sees what is coming.
async fn add_previews(app: &Arc<App>, id: &str) {
    let wanted: Vec<(String, PathBuf)> = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(id) else { return };
        t.files
            .iter()
            .filter(|f| {
                f.dto
                    .as_ref()
                    .is_some_and(|d| d.preview.is_none() && thumbnail::is_previewable(&d.file_type))
            })
            .filter_map(|f| Some((f.id.clone(), PathBuf::from(f.path.clone()?))))
            .take(thumbnail::MAX_PREVIEWS)
            .collect()
    };
    if wanted.is_empty() {
        return;
    }
    let previews = tokio::task::spawn_blocking(move || {
        wanted
            .into_iter()
            .filter_map(|(file, path)| thumbnail::data_url(&path).map(|p| (file, p)))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let mut transfers = app.transfers.lock().unwrap();
    if let Some(t) = transfers.get_mut(id) {
        for (file, preview) in previews {
            if let Some(dto) = t.file_mut(&file).and_then(|f| f.dto.as_mut()) {
                dto.preview = Some(preview);
            }
        }
    }
}

async fn run(app: &Arc<App>, id: &str, peer: &Peer, pin: Option<String>) -> Outcome {
    add_previews(app, id).await;
    let body = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(id) else {
            return Outcome::Failed("Transfer vanished".into());
        };
        let files: serde_json::Map<String, serde_json::Value> = t
            .files
            .iter()
            .filter_map(|f| f.dto.as_ref())
            .map(|dto| {
                (
                    dto.id.clone(),
                    serde_json::to_value(dto).expect("serializable"),
                )
            })
            .collect();
        json!({ "info": app.me().register_json(), "files": files })
    };
    let mut path = format!("{API}/prepare-upload");
    // A PIN only travels encrypted, to a certificate-pinned device.
    let pin = pin.filter(|p| !p.is_empty() && peer.info.is_https());
    if let Some(pin) = pin.as_deref() {
        path.push('?');
        path.push_str(&serde_urlencoded::to_string([("pin", pin)]).unwrap_or_default());
    }
    // Blocks until the receiver accepts or declines.
    let options = Options {
        connect_timeout: Duration::from_secs(6),
        response_timeout: None,
    };
    let response = match client::request(
        &app.identity,
        &peer.target(),
        Method::POST,
        &path,
        Body::json(&body),
        options,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => return err.into(),
    };
    match response.status {
        200 => {}
        204 => return Outcome::Done,
        403 => return Outcome::Declined,
        401 => {
            return Outcome::Failed(
                if pin.is_some() {
                    "Wrong PIN"
                } else {
                    "This device requires a PIN"
                }
                .into(),
            )
        }
        409 => return Outcome::Failed("The device is busy with another transfer".into()),
        429 => return Outcome::Failed("Too many attempts, try again later".into()),
        code => return Outcome::Failed(format!("Refused (HTTP {code})")),
    }
    let accepted: PrepareUploadResponse = match response.json() {
        Ok(r) => r,
        Err(err) => return err.into(),
    };
    {
        let mut transfers = app.transfers.lock().unwrap();
        if let Some(t) = transfers.get_mut(id) {
            t.remote_session_id = Some(accepted.session_id.clone());
            t.status = Status::Transferring;
            t.started = Some(now());
            for f in t.files.iter_mut() {
                if !accepted.files.contains_key(&f.id) {
                    f.status = FileStatus::Skipped;
                }
            }
        }
        transfers.emit(id, true);
    }
    // Upload in the order the user picked the files.
    let order: Vec<String> = {
        let transfers = app.transfers.lock().unwrap();
        transfers
            .get(id)
            .map(|t| t.files.iter().map(|f| f.id.clone()).collect())
            .unwrap_or_default()
    };
    // A few files at once hides the per-file handshake and save cost; receivers allow 8+ connections per IP.
    let uploads = order.into_iter().filter_map(|file_id| {
        accepted
            .files
            .get(&file_id)
            .map(|token| Ok((file_id, token)))
    });
    let session = &accepted.session_id;
    match futures_util::stream::iter(uploads)
        .try_for_each_concurrent(PARALLEL_UPLOADS, |(file_id, token)| async move {
            upload(app, id, peer, session, &file_id, token).await
        })
        .await
    {
        Ok(()) => Outcome::Done,
        Err(outcome) => outcome,
    }
}

async fn upload(
    app: &Arc<App>,
    id: &str,
    peer: &Peer,
    session: &str,
    file_id: &str,
    token: &str,
) -> Result<(), Outcome> {
    let (path, size, content_type, message) = {
        let mut transfers = app.transfers.lock().unwrap();
        let t = transfers
            .get_mut(id)
            .ok_or_else(|| Outcome::Failed("Transfer vanished".into()))?;
        let message = t.message.clone();
        let f = t
            .file_mut(file_id)
            .ok_or_else(|| Outcome::Failed("Unknown file".into()))?;
        f.status = FileStatus::Transferring;
        f.token = Some(token.to_string());
        let info = (f.path.clone(), f.size, f.file_type.clone(), message);
        transfers.emit(id, true);
        info
    };
    let stream: ByteStream = match (message, path) {
        (Some(text), _) => Box::pin(futures_util::stream::once(
            async move { Ok(Bytes::from(text)) },
        )),
        (None, Some(path)) => {
            let file = tokio::fs::File::open(&path)
                .await
                .map_err(|e| Outcome::Failed(format!("{path}: {e}")))?;
            Box::pin(ReaderStream::with_capacity(file, CHUNK))
        }
        (None, None) => return Err(Outcome::Failed("No data".into())),
    };
    let progress = {
        let (app, id, file_id) = (app.clone(), id.to_string(), file_id.to_string());
        Arc::new(move |sent: u64| {
            let mut transfers = app.transfers.lock().unwrap();
            if let Some(f) = transfers.get_mut(&id).and_then(|t| t.file_mut(&file_id)) {
                f.done = sent;
            }
            transfers.emit(&id, false);
        })
    };
    let query = serde_urlencoded::to_string([
        ("sessionId", session),
        ("fileId", file_id),
        ("token", token),
    ])
    .unwrap_or_default();
    let response = client::request(
        &app.identity,
        &peer.target(),
        Method::POST,
        &format!("{API}/upload?{query}"),
        Body::Stream {
            len: size,
            content_type,
            stream: client::with_progress(stream, progress),
        },
        Options {
            connect_timeout: Duration::from_secs(6),
            response_timeout: None,
        },
    )
    .await?;

    let mut transfers = app.transfers.lock().unwrap();
    let ok = matches!(response.status, 200 | 204);
    if let Some(f) = transfers.get_mut(id).and_then(|t| t.file_mut(file_id)) {
        if ok {
            f.status = FileStatus::Done;
            f.done = f.size;
        } else {
            f.status = FileStatus::Failed;
        }
    }
    transfers.emit(id, true);
    match response.status {
        200 | 204 => Ok(()),
        403 | 409 => Err(Outcome::Failed(
            "The receiver cancelled the transfer".into(),
        )),
        422 => Err(Outcome::Failed(
            "The file was damaged in transit (checksum mismatch)".into(),
        )),
        code => Err(Outcome::Failed(format!("Upload refused (HTTP {code})"))),
    }
}

async fn cancel_remote(app: &App, peer: &Peer, session: &str) {
    let query = serde_urlencoded::to_string([("sessionId", session)]).unwrap_or_default();
    let options = Options {
        connect_timeout: Duration::from_secs(3),
        response_timeout: Some(Duration::from_secs(5)),
    };
    let _ = client::request(
        &app.identity,
        &peer.target(),
        Method::POST,
        &format!("{API}/cancel?{query}"),
        Body::Empty,
        options,
    )
    .await;
}
