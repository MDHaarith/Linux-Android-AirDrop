//! Transfer bookkeeping shared by the receiver, the sender and the UI.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::EventBus;
use crate::protocol::FileDto;

const HISTORY_LIMIT: usize = 150;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Waiting for the receiver to accept.
    Waiting,
    Transferring,
    Done,
    Declined,
    Cancelled,
    Failed,
}

impl Status {
    pub fn is_final(self) -> bool {
        !matches!(self, Status::Waiting | Status::Transferring)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Queued,
    Transferring,
    Done,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    pub id: String,
    pub name: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub file_type: String,
    /// Source path when sending, destination path once received.
    pub path: Option<String>,
    pub done: u64,
    pub status: FileStatus,
    #[serde(skip)]
    pub dto: Option<FileDto>,
    #[serde(skip)]
    pub token: Option<String>,
}

impl FileState {
    pub fn new(dto: FileDto, path: Option<String>) -> Self {
        Self {
            id: dto.id.clone(),
            name: dto.file_name.clone(),
            size: dto.size,
            file_type: dto.file_type.clone(),
            path,
            done: 0,
            status: FileStatus::Queued,
            dto: Some(dto),
            token: None,
        }
    }
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: String,
    pub direction: Direction,
    /// Snapshot of the peer (alias, deviceType, …) for display.
    pub peer: Value,
    pub status: Status,
    pub message: Option<String>,
    pub error: Option<String>,
    pub created: f64,
    pub started: Option<f64>,
    pub finished: Option<f64>,
    pub files: Vec<FileState>,
    #[serde(skip)]
    pub peer_id: String,
    #[serde(skip)]
    pub peer_ip: Option<IpAddr>,
    /// LocalSend session we issued (incoming).
    #[serde(skip)]
    pub session_id: Option<String>,
    /// LocalSend session the receiver issued (outgoing).
    #[serde(skip)]
    pub remote_session_id: Option<String>,
    /// Temporary files deleted when the transfer ends.
    #[serde(skip)]
    pub cleanup: Vec<PathBuf>,
    /// Accepted without asking because it came from a trusted device.
    #[serde(skip)]
    pub auto_accepted: bool,
    /// The sender proved who it is (TLS client certificate or the browser
    /// pairing key). Only proven senders get the trusted-device treatment.
    #[serde(skip)]
    pub peer_verified: bool,
    /// Holds a secret (a Wi‑Fi password): the message is not saved to disk.
    #[serde(skip)]
    pub sensitive: bool,
    #[serde(skip)]
    last_emit: Option<Instant>,
}

impl Transfer {
    pub fn new(direction: Direction, peer_id: &str, peer: Value, files: Vec<FileState>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            direction,
            peer,
            status: Status::Waiting,
            message: None,
            error: None,
            created: now(),
            started: None,
            finished: None,
            files,
            peer_id: peer_id.to_string(),
            peer_ip: None,
            session_id: None,
            remote_session_id: None,
            cleanup: Vec::new(),
            auto_accepted: false,
            peer_verified: false,
            sensitive: false,
            last_emit: None,
        }
    }

    pub fn total(&self) -> u64 {
        self.files
            .iter()
            .filter(|f| f.status != FileStatus::Skipped)
            .fold(0u64, |total, f| total.saturating_add(f.size))
    }

    pub fn done_bytes(&self) -> u64 {
        self.files
            .iter()
            .filter(|f| f.status != FileStatus::Skipped)
            .map(|f| f.done)
            .sum()
    }

    pub fn file_mut(&mut self, id: &str) -> Option<&mut FileState> {
        self.files.iter_mut().find(|f| f.id == id)
    }

    pub fn file(&self, id: &str) -> Option<&FileState> {
        self.files.iter().find(|f| f.id == id)
    }

    pub fn to_json(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("serializable");
        let elapsed = self
            .started
            .map(|s| self.finished.unwrap_or_else(now) - s)
            .unwrap_or(0.0);
        let speed = if elapsed > 0.2 {
            self.done_bytes() as f64 / elapsed
        } else {
            0.0
        };
        // While a request waits for an answer, show what is coming.
        if self.status == Status::Waiting && self.direction == Direction::In {
            if let Some(files) = value["files"].as_array_mut() {
                for (json, file) in files.iter_mut().zip(&self.files) {
                    let preview = file.dto.as_ref().and_then(|d| d.preview.as_deref());
                    if let Some(thumb) = preview.and_then(crate::thumbnail::accept) {
                        json["thumb"] = thumb.into();
                    }
                }
            }
        }
        value["total"] = json!(self.total());
        value["done"] = json!(self.done_bytes());
        value["speed"] = json!(speed);
        value
    }
}

pub struct Transfers {
    items: HashMap<String, Transfer>,
    order: Vec<String>,
    history_path: PathBuf,
    bus: EventBus,
}

impl Transfers {
    pub fn new(bus: EventBus, history_path: PathBuf) -> Self {
        let mut store = Self {
            items: HashMap::new(),
            order: Vec::new(),
            history_path,
            bus,
        };
        store.load_history();
        store
    }

    fn load_history(&mut self) {
        let Ok(text) = std::fs::read_to_string(&self.history_path) else {
            return;
        };
        match serde_json::from_str::<Vec<Transfer>>(&text) {
            Ok(list) => {
                for mut transfer in list {
                    if !transfer.status.is_final() {
                        transfer.status = Status::Failed;
                        transfer.error = Some("Interrupted".into());
                    }
                    self.order.push(transfer.id.clone());
                    self.items.insert(transfer.id.clone(), transfer);
                }
            }
            Err(err) => tracing::warn!("Ignoring unreadable history: {err}"),
        }
    }

    fn save_history(&self) {
        let finals: Vec<Transfer> = self
            .order
            .iter()
            .filter_map(|id| self.items.get(id))
            .filter(|t| t.status.is_final())
            .map(|t| {
                let mut t = t.clone();
                if t.sensitive {
                    t.message = Some("(not saved: it contained a password)".into());
                }
                t
            })
            .collect();
        let start = finals.len().saturating_sub(HISTORY_LIMIT);
        match serde_json::to_vec(&finals[start..]) {
            Ok(bytes) => {
                if let Err(err) = crate::config::write_private(&self.history_path, &bytes) {
                    tracing::warn!("Could not save history: {err}");
                }
            }
            Err(err) => tracing::warn!("Could not serialize history: {err}"),
        }
    }

    pub fn add(&mut self, transfer: Transfer) -> String {
        let id = transfer.id.clone();
        self.bus.emit("transfer", transfer.to_json());
        self.order.push(id.clone());
        self.items.insert(id.clone(), transfer);
        // Forget the oldest finished transfers.
        while self.order.len() > HISTORY_LIMIT + 20 {
            let Some(pos) = self
                .order
                .iter()
                .position(|id| self.items[id].status.is_final())
            else {
                break;
            };
            let old = self.order.remove(pos);
            self.items.remove(&old);
        }
        id
    }

    pub fn get(&self, id: &str) -> Option<&Transfer> {
        self.items.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Transfer> {
        self.items.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Transfer> {
        self.order.iter().filter_map(|id| self.items.get(id))
    }

    /// The LocalSend session currently receiving (LocalSend allows one at a time).
    pub fn active_incoming(&self) -> Option<&Transfer> {
        self.iter().find(|t| {
            t.direction == Direction::In && t.session_id.is_some() && !t.status.is_final()
        })
    }

    /// Notifies the UI, rate-limited unless `force`.
    pub fn emit(&mut self, id: &str, force: bool) {
        let Some(transfer) = self.items.get_mut(id) else {
            return;
        };
        let now = Instant::now();
        if !force
            && transfer
                .last_emit
                .is_some_and(|t| now.duration_since(t) < PROGRESS_INTERVAL)
        {
            return;
        }
        transfer.last_emit = Some(now);
        self.bus.emit("transfer", transfer.to_json());
    }

    /// Moves a transfer to a final state. Returns false if it already was final.
    pub fn finish(&mut self, id: &str, status: Status, error: Option<String>) -> bool {
        let Some(transfer) = self.items.get_mut(id) else {
            return false;
        };
        if transfer.status.is_final() {
            return false;
        }
        transfer.status = status;
        transfer.error = error;
        transfer.finished = Some(now());
        for path in transfer.cleanup.drain(..) {
            let _ = std::fs::remove_file(&path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent); // staging dirs are per-file
            }
        }
        self.emit(id, true);
        self.save_history();
        true
    }

    pub fn clear_history(&mut self) {
        let finished: Vec<String> = self
            .items
            .values()
            .filter(|t| t.status.is_final())
            .map(|t| t.id.clone())
            .collect();
        for id in &finished {
            self.items.remove(id);
        }
        self.order.retain(|id| self.items.contains_key(id));
        self.save_history();
        self.bus.emit("history-cleared", Value::Null);
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.iter().map(Transfer::to_json).collect()
    }
}
