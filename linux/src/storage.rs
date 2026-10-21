//! Writing received files to the download folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::app::App;
use crate::protocol::{parse_iso, safe_relative_path, FileDto};
use crate::transfers::{FileStatus, Status};

#[derive(Debug)]
pub enum SaveError {
    /// The transfer was cancelled while receiving.
    Aborted,
    ChecksumMismatch,
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Aborted => write!(f, "cancelled"),
            SaveError::ChecksumMismatch => write!(f, "checksum mismatch"),
            SaveError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for SaveError {
    fn from(err: std::io::Error) -> Self {
        SaveError::Io(err.to_string())
    }
}

/// Destination names already claimed by uploads in progress.
#[derive(Default)]
pub struct Reservations(Mutex<HashSet<PathBuf>>);

impl Reservations {
    fn claim(&self, path: &Path) -> PathBuf {
        let mut names = self.0.lock().unwrap();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut candidate = path.to_path_buf();
        let mut n = 1;
        while names.contains(&candidate) || candidate.exists() {
            candidate = path.with_file_name(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        names.insert(candidate.clone());
        candidate
    }

    fn release(&self, path: &Path) {
        self.0.lock().unwrap().remove(path);
    }
}

/// Free bytes on the filesystem holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    let stat = nix::sys::statvfs::statvfs(path).ok()?;
    Some(stat.blocks_available() as u64 * stat.fragment_size() as u64)
}

/// Cleans up after an upload that did not complete (also on client disconnect).
struct UploadGuard {
    app: Arc<App>,
    transfer_id: String,
    file_id: String,
    part: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        self.app.reservations.release(&self.target);
        if self.committed {
            return;
        }
        let _ = std::fs::remove_file(&self.part);
        {
            let mut transfers = self.app.transfers.lock().unwrap();
            if let Some(f) = transfers
                .get_mut(&self.transfer_id)
                .and_then(|t| t.file_mut(&self.file_id))
            {
                f.status = FileStatus::Failed;
            }
        }
        finish_if_complete(&self.app, &self.transfer_id);
    }
}

/// Streams one received file into the download folder.
///
/// Data goes to a hidden `.part` file that is renamed when complete, so a
/// half-received file never looks finished.
pub async fn save_stream<S, E>(
    app: &Arc<App>,
    transfer_id: &str,
    dto: &FileDto,
    mut stream: S,
) -> Result<PathBuf, SaveError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let download_dir = app.settings().download_dir.clone();
    tokio::fs::create_dir_all(&download_dir).await?;
    let download_dir = tokio::fs::canonicalize(&download_dir).await?;
    let relative = safe_relative_path(&dto.file_name);
    let wanted = download_dir.join(&relative);
    let parent = wanted.parent().unwrap_or(&download_dir).to_path_buf();
    tokio::fs::create_dir_all(&parent).await?;
    // Refuse to follow symlinked folders out of the download folder.
    let parent = tokio::fs::canonicalize(&parent).await?;
    if !parent.starts_with(&download_dir) {
        return Err(SaveError::Io("invalid file name".into()));
    }
    let file_name = wanted
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "file".into());
    let target = app.reservations.claim(&parent.join(file_name));
    let part = target.with_file_name(format!(
        ".{}.airbridge-part",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let mut guard = UploadGuard {
        app: app.clone(),
        transfer_id: transfer_id.to_string(),
        file_id: dto.id.clone(),
        part: part.clone(),
        target: target.clone(),
        committed: false,
    };

    {
        let mut transfers = app.transfers.lock().unwrap();
        if let Some(f) = transfers
            .get_mut(transfer_id)
            .and_then(|t| t.file_mut(&dto.id))
        {
            f.status = FileStatus::Transferring;
            f.done = 0;
        }
        transfers.emit(transfer_id, true);
    }

    let mut file = tokio::fs::File::create(&part).await?;
    let mut hasher = dto.sha256.as_ref().map(|_| Sha256::new());
    let mut received = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| SaveError::Io(e.to_string()))?;
        file.write_all(&chunk).await?;
        if let Some(h) = hasher.as_mut() {
            h.update(&chunk);
        }
        received += chunk.len() as u64;
        let mut transfers = app.transfers.lock().unwrap();
        match transfers.get_mut(transfer_id) {
            Some(t) if !t.status.is_final() => {
                if let Some(f) = t.file_mut(&dto.id) {
                    f.done = received;
                }
            }
            _ => return Err(SaveError::Aborted),
        }
        transfers.emit(transfer_id, false);
    }
    file.flush().await?;
    drop(file);

    if received != dto.size {
        return Err(SaveError::Io(format!(
            "expected {} bytes, received {received}",
            dto.size
        )));
    }
    if let (Some(hasher), Some(expected)) = (hasher, dto.sha256.as_ref()) {
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(SaveError::ChecksumMismatch);
        }
    }
    tokio::fs::rename(&part, &target).await?;
    let metadata = dto.metadata.clone().unwrap_or_default();
    if let Some(modified) = parse_iso(metadata.modified.as_deref()) {
        let accessed = parse_iso(metadata.accessed.as_deref()).unwrap_or(modified);
        let _ = filetime::set_file_times(
            &target,
            filetime::FileTime::from_system_time(accessed),
            filetime::FileTime::from_system_time(modified),
        );
    }
    guard.committed = true;

    {
        let mut transfers = app.transfers.lock().unwrap();
        if let Some(f) = transfers
            .get_mut(transfer_id)
            .and_then(|t| t.file_mut(&dto.id))
        {
            f.status = FileStatus::Done;
            f.done = f.size;
            f.path = Some(target.to_string_lossy().into_owned());
        }
        transfers.emit(transfer_id, true);
    }
    finish_if_complete(app, transfer_id);
    Ok(target)
}

/// Finishes an incoming transfer once every file reached a final state.
pub fn finish_if_complete(app: &Arc<App>, transfer_id: &str) {
    let status = {
        let transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get(transfer_id) else {
            return;
        };
        if t.status.is_final()
            || t.files
                .iter()
                .any(|f| matches!(f.status, FileStatus::Queued | FileStatus::Transferring))
        {
            return;
        }
        if t.files
            .iter()
            .all(|f| matches!(f.status, FileStatus::Done | FileStatus::Skipped))
        {
            Status::Done
        } else {
            Status::Failed
        }
    };
    let finished = {
        let error =
            (status == Status::Failed).then(|| "Some files could not be received".to_string());
        app.transfers
            .lock()
            .unwrap()
            .finish(transfer_id, status, error)
    };
    if finished && status == Status::Done {
        app.on_files_received(transfer_id);
    }
}
