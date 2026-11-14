//! Sending files and text to LocalSend peers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::TryStreamExt;
use hyper::Method;
use serde::{Deserialize, Serialize};
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
    /// Checksum from an earlier attempt, reused while the file is unchanged.
    pub hashed: Option<Hashed>,
}

/// A file's SHA-256 and the size and modification time it was taken at.
#[derive(Debug, Clone, PartialEq)]
pub struct Hashed {
    pub sha256: String,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

impl Hashed {
    fn still_valid(&self, path: &Path) -> bool {
        std::fs::metadata(path)
            .is_ok_and(|m| m.len() == self.size && m.modified().ok() == self.modified)
    }
}

/// A chosen file that cannot be read now, with the reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unavailable {
    pub path: String,
    pub name: String,
    pub reason: String,
}

/// Why a file cannot be sent, found without reading (or downloading) it.
fn unreadable(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // OFFLINE, RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS: an online-only cloud
        // file. Reading it would download it, or fail if OneDrive is not running.
        const ONLINE_ONLY: u32 = 0x1000 | 0x4_0000 | 0x40_0000;
        match std::fs::metadata(path) {
            Ok(m) if m.file_attributes() & ONLINE_ONLY != 0 => {
                return Some(
                    "Online-only file: choose \u{201c}Always keep on this device\u{201d} in OneDrive to send it"
                        .into(),
                )
            }
            Err(e) => return Some(e.to_string()),
            Ok(_) => {}
        }
    }
    std::fs::File::open(path).err().map(|e| e.to_string())
}

/// Splits the chosen files into those that can be read and those that cannot.
pub fn check_readable(items: Vec<SendItem>) -> (Vec<SendItem>, Vec<Unavailable>) {
    let mut readable = Vec::with_capacity(items.len());
    let mut unavailable = Vec::new();
    for item in items {
        match unreadable(&item.path) {
            None => readable.push(item),
            Some(reason) => unavailable.push(Unavailable {
                path: item.path.to_string_lossy().into_owned(),
                name: item.name,
                reason,
            }),
        }
    }
    (readable, unavailable)
}

/// Which files of the chosen folders to send, and under which folder name.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Selection {
    /// Only these kinds from folders: "photos", "videos", "audio", "documents".
    pub kinds: Vec<String>,
    /// Only these extensions from folders (no dot, any case), besides `kinds`.
    pub extensions: Vec<String>,
    /// Folder names skipped anywhere inside the chosen folders.
    pub exclude: Vec<String>,
    /// Everything arrives inside this folder; a single chosen folder takes
    /// this name (sending `D:\` as "D-drive" keeps its whole hierarchy).
    pub root: Option<String>,
}

const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "csv", "ppt", "pptx",
    "odp", "epub",
];
/// Never user content, and often unreadable (Windows drive roots).
const SYSTEM_FOLDERS: &[&str] = &["System Volume Information", "$RECYCLE.BIN"];

impl Selection {
    fn filters(&self) -> bool {
        !self.kinds.is_empty() || !self.extensions.is_empty()
    }

    fn wants_file(&self, path: &Path) -> bool {
        if !self.filters() {
            return true;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        self.extensions
            .iter()
            .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
            || self.kinds.iter().any(|kind| match kind.as_str() {
                "photos" => mime.type_() == "image",
                "videos" => mime.type_() == "video",
                "audio" => mime.type_() == "audio",
                "documents" => DOCUMENTS.contains(&ext.as_str()),
                _ => false,
            })
    }

    fn wants_folder(&self, name: &str) -> bool {
        !SYSTEM_FOLDERS
            .iter()
            .copied()
            .chain(self.exclude.iter().map(String::as_str))
            .any(|skip| skip.eq_ignore_ascii_case(name))
    }
}

/// Files are sent as-is; folders are sent recursively with relative names,
/// filtered by `selection`. Any requested path that cannot be read is an
/// error, so nothing the user picked is silently left out.
pub fn expand_paths(paths: &[PathBuf], selection: &Selection) -> Result<Vec<SendItem>, String> {
    fn walk(
        dir: &Path,
        prefix: &str,
        selection: &Selection,
        out: &mut Vec<SendItem>,
    ) -> Result<(), String> {
        let failed = |e: std::io::Error| format!("{}: {e}", dir.display());
        let mut entries = std::fs::read_dir(dir)
            .and_then(|d| d.collect::<std::io::Result<Vec<_>>>())
            .map_err(failed)?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let kind = entry.file_type().map_err(failed)?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let name = format!("{prefix}/{file_name}");
            if kind.is_dir() && selection.wants_folder(&file_name) {
                walk(&entry.path(), &name, selection, out)?;
            } else if kind.is_file() && selection.wants_file(&entry.path()) {
                out.push(SendItem {
                    path: entry.path(),
                    name,
                    temporary: false,
                    hashed: None,
                });
            }
        }
        Ok(())
    }
    let root = selection
        .root
        .as_deref()
        .map(|r| r.trim_matches(['/', '\\', ' ']))
        .filter(|r| !r.is_empty());
    let mut items = Vec::new();
    for path in paths {
        // A drive root has no file name: "D:\" becomes "D".
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                path.to_string_lossy()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect()
            });
        let name = if name.is_empty() { "file".into() } else { name };
        let meta = std::fs::metadata(path);
        let name = match root {
            Some(root) if paths.len() == 1 && meta.as_ref().is_ok_and(|m| m.is_dir()) => {
                root.to_string()
            }
            Some(root) => format!("{root}/{name}"),
            None => name,
        };
        match meta {
            Ok(meta) if meta.is_dir() => walk(path, &name, selection, &mut items)?,
            Ok(meta) if meta.is_file() => items.push(SendItem {
                path: path.clone(),
                name,
                temporary: false,
                hashed: None,
            }),
            Ok(_) => return Err(format!("{}: not a file or folder", path.display())),
            Err(err) => return Err(format!("{}: {err}", path.display())),
        }
    }
    Ok(items)
}

enum Outcome {
    Done,
    Declined,
    Failed(String),
    /// The device could not be reached: worth trying again when it is back.
    Lost(String),
}

impl From<PeerError> for Outcome {
    fn from(err: PeerError) -> Self {
        match err {
            PeerError::Connect(_) | PeerError::Timeout => Outcome::Lost(err.to_string()),
            _ => Outcome::Failed(err.to_string()),
        }
    }
}

pub fn send_files(
    app: &Arc<App>,
    peer: Peer,
    items: Vec<SendItem>,
    pin: Option<String>,
    verify: bool,
) -> Result<String, String> {
    if items.is_empty() {
        return Err("Nothing to send".into());
    }
    let mut files = Vec::with_capacity(items.len());
    for item in &items {
        let mut dto = FileDto::for_path(&item.path, &item.name)
            .map_err(|e| format!("{}: {e}", item.path.display()))?;
        let hashed = item.hashed.clone().filter(|h| h.still_valid(&item.path));
        dto.sha256 = hashed.as_ref().map(|h| h.sha256.clone());
        let mut file = FileState::new(dto, Some(item.path.to_string_lossy().into_owned()));
        file.hashed = hashed;
        files.push(file);
    }
    let mut transfer = Transfer::new(
        Direction::Out,
        peer.id(),
        app.registry.peer_json(&peer),
        files,
    );
    transfer.peer_ip = Some(peer.ip);
    transfer.verify = verify || app.settings().verify_sends;
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
        let mut retry = false;
        let auto_resume = app.settings().auto_resume;
        let cancelled_session = {
            let mut transfers = app.transfers.lock().unwrap();
            let remote = transfers
                .get(&transfer_id)
                .and_then(|t| t.remote_session_id.clone());
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
                // The receiver accepted: end its session too, or it stays busy
                // waiting for files that will never come.
                Some(Outcome::Failed(error)) => {
                    transfers.finish(&transfer_id, Status::Failed, Some(error));
                    remote
                }
                Some(Outcome::Lost(error)) => {
                    if let Some(t) = transfers.get_mut(&transfer_id) {
                        retry = auto_resume && !t.status.is_final();
                        t.reconnecting = retry;
                    }
                    transfers.finish(&transfer_id, Status::Failed, Some(error));
                    remote
                }
                None => {
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
        if retry {
            app.resume_when_back(transfer_id);
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

/// For a verified send: the SHA-256 of every file, before the offer, so the
/// receiver can check what arrives (a file changed meanwhile fails there too).
async fn hash_files(app: &Arc<App>, id: &str) -> Result<(), String> {
    let files: Vec<(String, PathBuf)> = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers
            .get(id)
            .filter(|t| t.verify && t.message.is_none())
        else {
            return Ok(());
        };
        t.files
            .iter()
            .filter(|f| f.hashed.is_none())
            .filter_map(|f| Some((f.id.clone(), PathBuf::from(f.path.clone()?))))
            .collect()
    };
    let (app, id) = (app.clone(), id.to_string());
    tokio::task::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut checked = 0u64;
        let mut buf = vec![0; CHUNK];
        for (file_id, path) in files {
            let failed = |e: std::io::Error| format!("{}: {e}", path.display());
            let mut file = std::fs::File::open(&path).map_err(failed)?;
            let meta = file.metadata().map_err(failed)?;
            let mut hasher = Sha256::new();
            loop {
                let n = file.read(&mut buf).map_err(failed)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                checked += n as u64;
                let mut transfers = app.transfers.lock().unwrap();
                match transfers.get_mut(&id) {
                    Some(t) if !t.status.is_final() => t.checked = checked,
                    _ => return Err("Cancelled".to_string()),
                }
                transfers.emit(&id, false);
            }
            let digest: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let mut transfers = app.transfers.lock().unwrap();
            if let Some(dto) = transfers
                .get_mut(&id)
                .and_then(|t| t.file_mut(&file_id))
                .and_then(|f| f.dto.as_mut())
            {
                dto.sha256 = Some(digest.clone());
            }
            if let Some(f) = transfers.get_mut(&id).and_then(|t| t.file_mut(&file_id)) {
                f.hashed = Some(Hashed {
                    sha256: digest,
                    size: meta.len(),
                    modified: meta.modified().ok(),
                });
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn run(app: &Arc<App>, id: &str, peer: &Peer, pin: Option<String>) -> Outcome {
    add_previews(app, id).await;
    if let Err(err) = hash_files(app, id).await {
        return Outcome::Failed(err);
    }
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
            let file = match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(e) => {
                    let mut transfers = app.transfers.lock().unwrap();
                    if let Some(f) = transfers.get_mut(id).and_then(|t| t.file_mut(file_id)) {
                        f.status = FileStatus::Failed;
                    }
                    return Err(Outcome::Failed(format!("{path}: {e}")));
                }
            };
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
            "A file changed while it was sent, or was damaged in transit (checksum mismatch)"
                .into(),
        )),
        code => Err(Outcome::Failed(format!("Upload refused (HTTP {code})"))),
    }
}

/// Whether the device answers at all (any HTTP reply counts).
pub async fn reachable(app: &App, peer: &Peer) -> bool {
    let options = Options {
        connect_timeout: Duration::from_secs(4),
        response_timeout: Some(Duration::from_secs(6)),
    };
    client::request(
        &app.identity,
        &peer.target(),
        Method::GET,
        &format!("{API}/info"),
        Body::Empty,
        options,
    )
    .await
    .is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_are_reported_not_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.bin");
        std::fs::write(&file, b"a").unwrap();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        std::fs::write(dir.path().join("folder/b.bin"), b"b").unwrap();
        let all = Selection::default();
        let items = expand_paths(&[file.clone(), dir.path().join("folder")], &all).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["a.bin", "folder/b.bin"]);
        let missing = dir.path().join("does-not-exist.bin");
        let err = expand_paths(&[file, missing, dir.path().join("folder")], &all).unwrap_err();
        assert!(err.contains("does-not-exist.bin"), "{err}");
    }

    #[test]
    fn folders_can_be_filtered_and_received_under_one_name() {
        let drive = tempfile::tempdir().unwrap();
        for path in [
            "Photos/2024/a.JPG",
            "Photos/notes.pdf",
            "Videos/clip.mp4",
            "Program Files/app/setup.exe",
            "Games/save.raw",
            "System Volume Information/x.jpg",
        ] {
            let path = drive.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        let selection = Selection {
            kinds: vec!["photos".into(), "documents".into()],
            extensions: vec![".raw".into()],
            exclude: vec!["program files".into()],
            root: Some("D-drive/".into()),
        };
        let items = expand_paths(&[drive.path().to_path_buf()], &selection).unwrap();
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "D-drive/Games/save.raw",
                "D-drive/Photos/2024/a.JPG",
                "D-drive/Photos/notes.pdf"
            ]
        );
        // Several chosen paths each keep their own name inside the folder.
        let file = drive.path().join("Videos/clip.mp4");
        let items = expand_paths(&[file.clone(), file], &selection).unwrap();
        assert_eq!(items[0].name, "D-drive/clip.mp4");
    }

    fn wait_final(app: &App, id: &str) -> Transfer {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(t) = app
                .transfers
                .lock()
                .unwrap()
                .get(id)
                .filter(|t| t.status.is_final())
            {
                return t.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "transfer {id} did not finish"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verified_send_then_resend_skips_what_already_arrived() {
        let settings = r#"{"notifications":false,"quickShare":false}"#;
        let (_a_home, a) = crate::app::test_app(settings);
        let (_b_home, b) = crate::app::test_app(settings);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut info = b.me();
        info.port = listener.local_addr().unwrap().port();
        let tls = Some(b.identity.server_config());
        tokio::spawn(crate::receiver::serve(b.clone(), listener, tls));
        let peer = a.registry.upsert(
            info,
            "127.0.0.1".parse().unwrap(),
            crate::peers::PeerKind::LocalSend,
            true,
        );
        // The receiving user accepts everything.
        let receiver = b.clone();
        tokio::spawn(async move {
            loop {
                let waiting: Vec<String> = receiver
                    .transfers
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|t| t.status == Status::Waiting)
                    .map(|t| t.id.clone())
                    .collect();
                for id in waiting {
                    receiver.decide(&id, true);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Album")).unwrap();
        std::fs::write(dir.path().join("Album/a.jpg"), vec![7u8; 300_000]).unwrap();
        std::fs::write(dir.path().join("Album/b.txt"), b"hello").unwrap();
        let items = expand_paths(&[dir.path().join("Album")], &Selection::default()).unwrap();

        let id = send_files(&a, peer.clone(), items.clone(), None, true).unwrap();
        let (a2, id2) = (a.clone(), id.clone());
        let sent = tokio::task::spawn_blocking(move || wait_final(&a2, &id2))
            .await
            .unwrap();
        assert_eq!(sent.status, Status::Done, "{:?}", sent.error);
        let received = b
            .transfers
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.direction == Direction::In)
            .cloned()
            .unwrap();
        assert!(received
            .files
            .iter()
            .all(|f| f.status == FileStatus::Done && f.verified));
        let downloads = b.settings().download_dir.clone();
        assert_eq!(
            std::fs::read(downloads.join("Album/b.txt")).unwrap(),
            b"hello"
        );

        // Sending the same files again (a resume after an unclear failure)
        // creates no copies: the receiver already has them.
        let id = send_files(&a, peer, items, None, false).unwrap();
        let (a2, id2) = (a.clone(), id.clone());
        let again = tokio::task::spawn_blocking(move || wait_final(&a2, &id2))
            .await
            .unwrap();
        assert_eq!(again.status, Status::Done, "{:?}", again.error);
        assert!(again.files.iter().all(|f| f.status == FileStatus::Skipped));
        assert!(!downloads.join("Album/b (1).txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_are_listed_before_sending() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.txt"), b"ok").unwrap();
        let locked = dir.path().join("locked.txt");
        std::fs::write(&locked, b"no").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            return; // running as root: nothing is unreadable
        }
        let items = expand_paths(&[dir.path().to_path_buf()], &Selection::default()).unwrap();
        let (readable, unavailable) = check_readable(items);
        assert_eq!(readable.len(), 1);
        assert!(readable[0].name.ends_with("/ok.txt"));
        assert_eq!(unavailable.len(), 1);
        assert!(unavailable[0].name.ends_with("/locked.txt"));
        assert!(!unavailable[0].reason.is_empty());
    }

    #[test]
    fn a_checksum_is_reused_only_while_the_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let hashed = Hashed {
            sha256: "x".into(),
            size: meta.len(),
            modified: meta.modified().ok(),
        };
        assert!(hashed.still_valid(&path));
        std::fs::write(&path, b"abcd").unwrap();
        assert!(!hashed.still_valid(&path));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_send_to_a_device_that_dropped_off_resumes_when_it_is_back() {
        let settings = r#"{"notifications":false,"quickShare":false}"#;
        let (_a_home, a) = crate::app::test_app(settings);
        let (_b_home, b) = crate::app::test_app(settings);
        // Nothing listens on the port yet: the device is away.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut info = b.me();
        info.port = port;
        let peer = a.registry.upsert(
            info,
            "127.0.0.1".parse().unwrap(),
            crate::peers::PeerKind::LocalSend,
            true,
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), vec![7u8; 200_000]).unwrap();
        let items = expand_paths(&[dir.path().join("a.jpg")], &Selection::default()).unwrap();
        let id = send_files(&a, peer, items, None, true).unwrap();
        let (a2, id2) = (a.clone(), id.clone());
        let lost = tokio::task::spawn_blocking(move || wait_final(&a2, &id2))
            .await
            .unwrap();
        assert_eq!(lost.status, Status::Failed);
        assert!(lost.reconnecting, "{:?}", lost.error);
        assert!(lost.files[0].hashed.is_some());

        // The device comes back and accepts.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        tokio::spawn(crate::receiver::serve(
            b.clone(),
            listener,
            Some(b.identity.server_config()),
        ));
        let receiver = b.clone();
        tokio::spawn(async move {
            loop {
                let waiting: Vec<String> = receiver
                    .transfers
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|t| t.status == Status::Waiting)
                    .map(|t| t.id.clone())
                    .collect();
                for id in waiting {
                    receiver.decide(&id, true);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let a2 = a.clone();
        let resumed = tokio::task::spawn_blocking(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                let next = a2
                    .transfers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|t| t.id != id && t.direction == Direction::Out)
                    .map(|t| t.id.clone());
                if let Some(next) = next {
                    return (
                        wait_final(&a2, &next),
                        a2.transfers.lock().unwrap().get(&id).cloned(),
                    );
                }
                assert!(std::time::Instant::now() < deadline, "never resumed");
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .await
        .unwrap();
        let (resumed, old) = resumed;
        assert_eq!(resumed.status, Status::Done, "{:?}", resumed.error);
        // The checksum from the first attempt was reused, not taken again.
        assert_eq!(resumed.checked, 0);
        assert!(!old.unwrap().reconnecting);
    }
}
