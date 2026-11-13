//! Writing received files to the download folder.

use std::collections::HashSet;
#[cfg(target_os = "linux")]
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

/// Partial files being written, also listed on disk so the ones a crash left
/// behind are removed at the next start. Only files this profile created are
/// ever listed, so another instance's uploads are never touched.
pub struct Partials {
    list: PathBuf,
    active: Mutex<HashSet<PathBuf>>,
}

impl Partials {
    /// Removes what the previous run left behind. Only the service that owns
    /// the profile (see `lock_profile`) may call this.
    pub fn new(list: PathBuf) -> Self {
        let leftovers: Vec<PathBuf> = std::fs::read(&list)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        for part in leftovers {
            let ours = part
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".airbridge-part"));
            if ours && std::fs::remove_file(&part).is_ok() {
                tracing::info!("Removed unfinished download {}", part.display());
            }
        }
        let _ = std::fs::remove_file(&list);
        Self {
            list,
            active: Mutex::default(),
        }
    }

    fn change(&self, change: impl FnOnce(&mut HashSet<PathBuf>)) {
        let mut active = self.active.lock().unwrap();
        change(&mut active);
        let saved = serde_json::to_vec(&*active)
            .map_err(std::io::Error::from)
            .and_then(|bytes| crate::config::write_private(&self.list, &bytes));
        if let Err(err) = saved {
            tracing::warn!("Could not record partial downloads: {err}");
        }
    }

    fn add(&self, part: &Path) {
        self.change(|active| {
            active.insert(part.to_path_buf());
        });
    }

    fn remove(&self, part: &Path) {
        self.change(|active| {
            active.remove(part);
        });
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
            Ok(meta) if real_directory(&meta) => {}
            Ok(_) => return Err(SaveError::Io("invalid file name".into())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&dir) {
                    Ok(()) => {}
                    // Created by a parallel upload of the same folder.
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        if !real_directory(&std::fs::symlink_metadata(&dir)?) {
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

fn real_directory(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points can redirect writes just like symlinks.
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    meta.is_dir() && !meta.file_type().is_symlink()
}

/// Opens the partial file without following symlinks and without reusing an
/// existing file (a leftover from a crash is replaced, not appended to).
#[cfg(target_os = "linux")]
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

#[cfg(windows)]
fn create_part(path: &Path) -> std::io::Result<std::fs::File> {
    // create_new rejects existing files, symlinks and reparse points atomically.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
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
                #[cfg(target_os = "linux")]
                std::fs::rename(part, &target)?;
                #[cfg(windows)]
                {
                    // FAT has no hard links; create_new still prevents replacing another file.
                    let mut output = match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)
                    {
                        Ok(output) => output,
                        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                            let next = app.reservations.claim(&target);
                            app.reservations.release(&target);
                            target = next;
                            continue;
                        }
                        Err(err) => return Err(err),
                    };
                    let copied = std::fs::File::open(part)
                        .and_then(|mut input| std::io::copy(&mut input, &mut output))
                        .and_then(|_| output.sync_all());
                    drop(output);
                    if let Err(err) = copied {
                        let _ = std::fs::remove_file(&target);
                        return Err(err);
                    }
                    std::fs::remove_file(part)?;
                }
                return Ok(target);
            }
        }
    }
    Err(std::io::Error::other("no free file name"))
}

/// Free bytes on the filesystem holding `path`.
#[cfg(target_os = "linux")]
pub fn free_space(path: &Path) -> Option<u64> {
    let stat = nix::sys::statvfs::statvfs(path).ok()?;
    Some(stat.blocks_available() as u64 * stat.fragment_size() as u64)
}

#[cfg(windows)]
pub fn free_space(_path: &Path) -> Option<u64> {
    // shortcut: no std disk-space API; add a native query when preflight space checks are needed.
    None
}

/// Cleans up after an upload that did not complete (also on client disconnect).
/// Created as soon as the file is marked as transferring, so every error exit
/// marks it failed and lets the session finish.
struct UploadGuard {
    app: Arc<App>,
    transfer_id: String,
    file_id: String,
    /// Set once our own partial file exists.
    part: Option<PathBuf>,
    target: Option<PathBuf>,
    committed: bool,
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        if let Some(target) = &self.target {
            self.app.reservations.release(target);
        }
        if let Some(part) = &self.part {
            if !self.committed {
                let _ = std::fs::remove_file(part);
            }
            self.app.partials.remove(part);
        }
        if self.committed {
            return;
        }
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
    let ended = {
        let mut transfers = app.transfers.lock().unwrap();
        let Some(t) = transfers.get_mut(transfer_id) else {
            return Err(SaveError::Aborted);
        };
        let ended = t.ended.clone();
        let Some(f) = t.file_mut(&dto.id) else {
            return Err(SaveError::Aborted);
        };
        if f.status == FileStatus::Transferring {
            return Err(SaveError::Io("this file is already being received".into()));
        }
        f.status = FileStatus::Transferring;
        f.done = 0;
        transfers.emit(transfer_id, true);
        ended
    };
    let mut guard = UploadGuard {
        app: app.clone(),
        transfer_id: transfer_id.to_string(),
        file_id: dto.id.clone(),
        part: None,
        target: None,
        committed: false,
    };
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
    guard.target = Some(target.clone());
    let part = target.with_file_name(format!(
        ".{}.airbridge-part",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    // A stale Windows partial must not block the next transfer of the same name.
    #[cfg(windows)]
    let part = part.with_extension(format!("{}.airbridge-part", uuid::Uuid::new_v4().simple()));

    // Recorded before it exists, so even a crash right after leaves no orphan.
    app.partials.add(&part);
    let mut file = match create_part(&part) {
        Ok(file) => tokio::fs::File::from_std(file),
        Err(err) => {
            app.partials.remove(&part);
            return Err(err.into());
        }
    };
    guard.part = Some(part.clone());
    let mut hasher = dto.sha256.as_ref().map(|_| Sha256::new());
    let mut received = 0u64;
    loop {
        // A cancelled session stops at once, even while the sender is silent.
        let next = tokio::select! {
            _ = ended.cancelled() => return Err(SaveError::Aborted),
            next = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => next
                .map_err(|_| SaveError::Io("the sender stopped sending".into()))?,
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::{Direction, FileState, Transfer};

    fn incoming(app: &Arc<App>, name: &str, size: u64) -> (String, FileDto) {
        let dto = FileDto {
            id: "f1".into(),
            file_name: name.into(),
            size,
            ..Default::default()
        };
        let mut t = Transfer::new(
            Direction::In,
            "peer",
            serde_json::Value::Null,
            vec![FileState::new(dto.clone(), None)],
        );
        t.session_id = Some("s".into());
        t.status = Status::Transferring;
        (app.transfers.lock().unwrap().add(t), dto)
    }

    fn parts(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".airbridge-part"))
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn partials_left_by_a_crash_are_removed_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("partials.json");
        let part = dir.path().join(".a.bin.airbridge-part");
        let finished = dir.path().join(".b.bin.airbridge-part");
        let not_a_part = dir.path().join("keep.bin");
        let partials = Partials::new(list.clone());
        for path in [&part, &finished, &not_a_part] {
            std::fs::write(path, b"x").unwrap();
            partials.add(path);
        }
        partials.remove(&finished);
        drop(partials); // the process dies without cleaning up
        Partials::new(list.clone());
        assert!(!part.exists());
        assert!(finished.exists() && not_a_part.exists());
        assert!(!list.exists());
    }

    #[tokio::test]
    async fn destination_errors_fail_the_session_and_cancel_stops_a_silent_sender() {
        let (_home, app) = crate::app::test_app(r#"{"notifications":false,"quickShare":false}"#);
        let downloads = app.settings().download_dir.clone();
        std::fs::create_dir_all(&downloads).unwrap();

        // A folder that exists as a file: refused, and the session must not stay busy.
        std::fs::write(downloads.join("taken"), b"").unwrap();
        let (id, dto) = incoming(&app, "taken/blocked.bin", 4);
        let data =
            futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"data"))]);
        assert!(matches!(
            save_stream(&app, &id, &dto, data).await,
            Err(SaveError::Io(_))
        ));
        {
            let transfers = app.transfers.lock().unwrap();
            assert_eq!(transfers.get(&id).unwrap().status, Status::Failed);
            assert!(transfers.active_incoming().is_none());
        }

        // A sender that goes quiet mid-file is stopped as soon as the user cancels.
        let (id, dto) = incoming(&app, "stalled.bin", 8);
        let data =
            futures_util::stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(b"half"))])
                .chain(futures_util::stream::pending());
        let upload = tokio::spawn({
            let (app, id) = (app.clone(), id.clone());
            async move { save_stream(&app, &id, &dto, data).await }
        });
        while parts(&downloads) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        app.cancel(&id);
        let result = tokio::time::timeout(Duration::from_secs(5), upload)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(SaveError::Aborted)));
        assert_eq!(parts(&downloads), 0);
    }

    #[test]
    fn folders_stay_inside_root_and_existing_parts_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        assert!(create_folders(root.path(), Path::new("../outside")).is_err());
        assert!(create_folders(root.path(), Path::new("nested/folder")).is_ok());
        let part = root.path().join("part");
        std::fs::write(&part, b"keep").unwrap();
        #[cfg(windows)]
        {
            assert!(create_part(&part).is_err());
            assert_eq!(std::fs::read(&part).unwrap(), b"keep");
        }
        #[cfg(target_os = "linux")]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
            assert!(create_folders(root.path(), Path::new("link/escape")).is_err());
            std::os::unix::fs::symlink(&part, root.path().join("part-link")).unwrap();
            create_part(&root.path().join("part-link")).unwrap();
            assert_eq!(std::fs::read(&part).unwrap(), b"keep");
        }
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_not_receive_folders() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        // Fixed test command; no sender-controlled path ever reaches a shell.
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(root.path().join("junction"))
            .arg(outside.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(create_part(&root.path().join("junction")).is_err());
        assert!(create_folders(root.path(), Path::new("junction/escape")).is_err());
        assert!(!outside.path().join("escape").exists());
    }
}
