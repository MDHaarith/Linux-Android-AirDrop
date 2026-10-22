//! Writing received files to the download folder.

use std::collections::HashSet;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// A sender that goes quiet this long mid-file is dropped.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Creates the folders of a received path one by one inside `root`, refusing
/// any component that already exists as something other than a real folder
/// (a symlink planted in the download folder must not lead outside it).
fn create_folders(root: &Path, relative: &Path) -> Result<PathBuf, SaveError> {
    let mut dir = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(SaveError::Io("invalid file name".into()));
        };
        dir.push(name);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(SaveError::Io("invalid file name".into())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&dir) {
                    Ok(()) => {}
                    // Created by a parallel upload of the same folder.
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        if !std::fs::symlink_metadata(&dir)?.is_dir() {
                            return Err(SaveError::Io("invalid file name".into()));
                        }
                    }
                    Err(err) => return Err(err.into()),
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(dir)
}

/// Opens the partial file without following symlinks and without reusing an
/// existing file (a leftover from a crash is replaced, not appended to).
fn create_part(path: &Path) -> std::io::Result<std::fs::File> {
    let open = || {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
    };
    match open() {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(path)?;
            open()
        }
        other => other,
    }
}

/// Moves the finished file into place without replacing anything that
/// appeared there meanwhile; returns where it landed.
fn commit(app: &App, part: &Path, target: PathBuf) -> std::io::Result<PathBuf> {
    let mut target = target;
    for _ in 0..100 {
        match std::fs::hard_link(part, &target) {
            Ok(()) => {
                std::fs::remove_file(part)?;
                return Ok(target);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let next = app.reservations.claim(&target);
                app.reservations.release(&target);
                target = next;
            }
            // File systems without hard links (FAT on a USB stick).
            Err(_) => {
                std::fs::rename(part, &target)?;
                return Ok(target);
            }
        }
    }
    Err(std::io::Error::other("no free file name"))
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
    // One upload per file at a time: a second request for the same file
    // would write the data twice.
    {
        let mut transfers = app.transfers.lock().unwrap();
        let Some(f) = transfers
            .get_mut(transfer_id)
            .and_then(|t| t.file_mut(&dto.id))
        else {
            return Err(SaveError::Aborted);
        };
        if f.status == FileStatus::Transferring {
            return Err(SaveError::Io("this file is already being received".into()));
        }
        f.status = FileStatus::Transferring;
        f.done = 0;
        transfers.emit(transfer_id, true);
    }
    let download_dir = app.settings().download_dir.clone();
    tokio::fs::create_dir_all(&download_dir).await?;
    let download_dir = tokio::fs::canonicalize(&download_dir).await?;
    let relative = safe_relative_path(&dto.file_name);
    let folders = relative.parent().map(Path::to_path_buf).unwrap_or_default();
    let parent = create_folders(&download_dir, &folders)?;
    // Refuse to follow symlinked folders out of the download folder.
    if !tokio::fs::canonicalize(&parent)
        .await?
        .starts_with(&download_dir)
    {
        return Err(SaveError::Io("invalid file name".into()));
    }
    let file_name = relative
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

    let mut file = tokio::fs::File::from_std(create_part(&part)?);
    let mut hasher = dto.sha256.as_ref().map(|_| Sha256::new());
    let mut received = 0u64;
    loop {
        let next = tokio::time::timeout(STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| SaveError::Io("the sender stopped sending".into()))?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| SaveError::Io(e.to_string()))?;
        // Never write more than was announced (and accepted).
        if received + chunk.len() as u64 > dto.size {
            return Err(SaveError::Io(format!(
                "more data than the announced {} bytes",
                dto.size
            )));
        }
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
    let landed = {
        let (app, part, target) = (app.clone(), part.clone(), target.clone());
        tokio::task::spawn_blocking(move || commit(&app, &part, target))
            .await
            .map_err(|e| SaveError::Io(e.to_string()))??
    };
    let target = landed;
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
