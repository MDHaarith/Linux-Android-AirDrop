//! Application state shared by all servers, discovery and the UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, RwLockReadGuard};
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::companion::Hub;
use crate::config::{self, Settings};
use crate::desktop::{self, Notifier};
use crate::discovery::Discovery;
use crate::events::EventBus;
use crate::identity::Identity;
use crate::limits::RateLimit;
use crate::net;
use crate::peers::{PeerKind, Registry};
use crate::protocol::{DeviceInfo, PROTOCOL_VERSION};
use crate::quickshare;
use crate::sender::{self, SendItem};
use crate::storage::Reservations;
use crate::transfers::{Direction, Status, Transfers};

const DECISION_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a window launch code stays valid (it also works only once).
const LAUNCH_CODE_TTL: Duration = Duration::from_secs(60);

pub struct UiInfo {
    pub port: u16,
    pub token: String,
}

pub struct App {
    settings: RwLock<Settings>,
    pub identity: Identity,
    pub bus: EventBus,
    pub registry: Registry,
    pub transfers: Mutex<Transfers>,
    pub notifier: Notifier,
    pub companion: Hub,
    pub reservations: Reservations,
    /// Files dropped into the window, copied to the cache until sent.
    pub staged: Mutex<HashMap<String, SendItem>>,
    /// Wrong PINs: a few per address, and a cap for everyone together so
    /// guessing from many addresses does not help.
    pub pin_failures: RateLimit,
    /// Requests that reach the user (prompts, messages), per sender.
    pub requests: RateLimit,
    decisions: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// One-time codes in window URLs, traded for the session cookie.
    launch_codes: Mutex<Vec<(String, std::time::Instant)>>,
    cancels: Mutex<HashMap<String, CancellationToken>>,
    /// Port of the LocalSend HTTPS server actually bound.
    pub port: AtomicU16,
    /// Port of the web companion actually bound (0 = not running).
    pub web_port: AtomicU16,
    pub ui: OnceLock<UiInfo>,
    /// Open desktop UI windows (WebSocket connections).
    pub ui_clients: AtomicUsize,
    pub discovery: OnceLock<Arc<Discovery>>,
    /// Google Quick Share interoperability, while enabled.
    pub quick_share: Mutex<Option<Arc<quickshare::Service>>>,
    pub shutdown: CancellationToken,
}

impl App {
    pub fn new() -> Result<Arc<Self>> {
        let settings = Settings::load();
        let identity = Identity::load_or_create(&config::config_dir().join("identity"))?;
        let bus = EventBus::default();
        let registry = Registry::new(bus.clone(), &settings.trusted);
        let transfers = Transfers::new(bus.clone(), config::data_dir().join("history.json"));
        let port = settings.port;
        Ok(Arc::new(Self {
            settings: RwLock::new(settings),
            identity,
            bus,
            registry,
            transfers: Mutex::new(transfers),
            notifier: Notifier::default(),
            companion: Hub::default(),
            reservations: Reservations::default(),
            staged: Mutex::new(HashMap::new()),
            pin_failures: RateLimit::new(5, 20, Duration::from_secs(600)),
            requests: RateLimit::new(20, 120, Duration::from_secs(60)),
            decisions: Mutex::new(HashMap::new()),
            launch_codes: Mutex::new(Vec::new()),
            cancels: Mutex::new(HashMap::new()),
            port: AtomicU16::new(port),
            web_port: AtomicU16::new(0),
            ui: OnceLock::new(),
            ui_clients: AtomicUsize::new(0),
            discovery: OnceLock::new(),
            quick_share: Mutex::new(None),
            shutdown: CancellationToken::new(),
        }))
    }

    pub fn settings(&self) -> RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap()
    }

    pub fn update_settings<R>(&self, change: impl FnOnce(&mut Settings) -> R) -> R {
        let mut settings = self.settings.write().unwrap();
        let result = change(&mut settings);
        if let Err(err) = settings.save() {
            tracing::warn!("Could not save settings: {err}");
        }
        drop(settings);
        self.bus.emit("settings", self.settings().public_json());
        result
    }

    /// How we describe ourselves to other devices.
    pub fn me(&self) -> DeviceInfo {
        let settings = self.settings();
        DeviceInfo {
            alias: settings.alias.clone(),
            version: PROTOCOL_VERSION.into(),
            device_model: Some(settings.device_model.clone()),
            device_type: Some("desktop".into()),
            fingerprint: self.identity.fingerprint.clone(),
            port: self.port.load(Ordering::Relaxed),
            protocol: if settings.https { "https" } else { "http" }.into(),
            download: false,
        }
    }

    /// The address that opens the window. It carries a one-time code rather
    /// than the API token, since URLs end up in process lists, logs and
    /// browser history: once used (or after a minute) it is worthless.
    pub fn ui_url(&self) -> Option<String> {
        use rand::distributions::{Alphanumeric, DistString};
        let ui = self.ui.get()?;
        let code = Alphanumeric.sample_string(&mut rand::thread_rng(), 32);
        let mut codes = self.launch_codes.lock().unwrap();
        codes.retain(|(_, at)| at.elapsed() < LAUNCH_CODE_TTL);
        codes.push((code.clone(), std::time::Instant::now()));
        Some(format!("http://127.0.0.1:{}/?code={code}", ui.port))
    }

    /// Uses up a launch code; true if it was valid.
    pub fn redeem_launch_code(&self, code: &str) -> bool {
        let mut codes = self.launch_codes.lock().unwrap();
        codes.retain(|(_, at)| at.elapsed() < LAUNCH_CODE_TTL);
        match codes
            .iter()
            .position(|(c, _)| crate::receiver::constant_time_eq(c.as_bytes(), code.as_bytes()))
        {
            Some(i) => {
                codes.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn companion_url(&self) -> Option<String> {
        let settings = self.settings();
        let port = self.web_port.load(Ordering::Relaxed);
        if !settings.web_companion || port == 0 {
            return None;
        }
        let ip = net::primary_ipv4()?;
        Some(format!("http://{ip}:{port}/#k={}", settings.web_key))
    }

    pub fn state_json(&self) -> Value {
        let me = self.me();
        let discovery = self.discovery.get();
        json!({
            "me": {
                "alias": me.alias,
                "deviceModel": me.device_model,
                "fingerprint": me.fingerprint,
                "ip": net::primary_ipv4().map(|ip| ip.to_string()),
                "port": me.port,
                "version": env!("CARGO_PKG_VERSION"),
            },
            "peers": self.registry.snapshot(),
            "transfers": self.transfers.lock().unwrap().snapshot(),
            "settings": self.settings().public_json(),
            "companion": { "url": self.companion_url() },
            "multicast": discovery.is_some_and(|d| d.multicast_ok()),
            "scanning": discovery.is_some_and(|d| d.scanning.load(Ordering::Relaxed)),
        })
    }

    // ------------------------------------------------------------ decisions

    /// Asks the user whether to accept an incoming transfer (window and/or
    /// notification). Trusted devices are accepted automatically.
    pub async fn ask_accept(self: &Arc<Self>, id: &str) -> bool {
        let (peer_id, verified, alias, summary_files, pin, phrase, preview) = {
            let transfers = self.transfers.lock().unwrap();
            let Some(t) = transfers.get(id) else {
                return false;
            };
            let alias = t
                .peer
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or("A device")
                .to_string();
            (
                t.peer_id.clone(),
                t.peer_verified,
                alias,
                describe_files(
                    &t.files
                        .iter()
                        .map(|f| (f.name.as_str(), f.size))
                        .collect::<Vec<_>>(),
                ),
                t.peer
                    .get("pin")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                share_phrase(
                    &t.files
                        .iter()
                        .map(|f| (f.name.as_str(), f.file_type.as_str()))
                        .collect::<Vec<_>>(),
                ),
                t.files
                    .iter()
                    .find_map(|f| f.dto.as_ref().and_then(|d| d.preview.clone())),
            )
        };
        // Your own devices, when they prove it with their certificate.
        if self.settings().auto_accept_trusted && verified && self.registry.is_trusted(&peer_id) {
            // Your own devices: no question, like AirDrop on the same Apple ID.
            if let Some(t) = self.transfers.lock().unwrap().get_mut(id) {
                t.auto_accepted = true;
            }
            return true;
        }
        // The photo itself in the notification, when the sender included one
        // (re-encoded by us first; the original bytes come from the network).
        let image = match preview {
            Some(preview) => {
                tokio::task::spawn_blocking(move || crate::thumbnail::reencode(&preview))
                    .await
                    .ok()
                    .flatten()
                    .and_then(|jpeg| crate::thumbnail::save_for_notification(id, &jpeg))
            }
            None => None,
        };

        let (tx, rx) = oneshot::channel();
        self.decisions.lock().unwrap().insert(id.to_string(), tx);
        self.bus.emit("incoming", json!({ "id": id }));

        let notify_enabled = self.settings().notifications;
        let question = if notify_enabled && self.notifier.available().await {
            let (app, id) = (self.clone(), id.to_string());
            let title = format!("“{alias}” would like to share {phrase}");
            let image = image.clone();
            // Quick Share shows the same PIN on the phone.
            let body = match &pin {
                Some(pin) => format!("{} · PIN {pin}", summary_files.1),
                None => summary_files.1.clone(),
            };
            Some(tokio::spawn(async move {
                let answer = app
                    .notifier
                    .ask(
                        &title,
                        &body,
                        image.as_deref(),
                        &[
                            ("accept", "Accept"),
                            ("decline", "Decline"),
                            ("default", "Show"),
                        ],
                    )
                    .await;
                match answer.as_deref() {
                    Some("accept") => {
                        app.decide(&id, true);
                    }
                    Some("decline") => {
                        app.decide(&id, false);
                    }
                    Some(_) => app.open_window(),
                    None => {}
                }
            }))
        } else {
            None
        };
        if question.is_none() && self.ui_clients.load(Ordering::Relaxed) == 0 {
            self.open_window();
        }

        struct Cleanup<'a> {
            app: &'a App,
            id: &'a str,
            question: Option<tokio::task::JoinHandle<()>>,
            image: Option<PathBuf>,
        }
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                self.app.decisions.lock().unwrap().remove(self.id);
                if let Some(task) = self.question.take() {
                    task.abort(); // closes the notification
                }
                if let Some(image) = self.image.take() {
                    let _ = std::fs::remove_file(image);
                }
            }
        }
        let _cleanup = Cleanup {
            app: self,
            id,
            question,
            image,
        };
        matches!(
            tokio::time::timeout(DECISION_TIMEOUT, rx).await,
            Ok(Ok(true))
        )
    }

    /// Answers a pending incoming request. Returns false if none was pending.
    pub fn decide(&self, id: &str, accept: bool) -> bool {
        match self.decisions.lock().unwrap().remove(id) {
            Some(tx) => tx.send(accept).is_ok(),
            None => false,
        }
    }

    pub fn finish(&self, id: &str, status: Status, error: Option<String>) -> bool {
        self.decisions.lock().unwrap().remove(id);
        self.transfers.lock().unwrap().finish(id, status, error)
    }

    pub fn register_cancel(&self, id: &str, token: CancellationToken) {
        self.cancels.lock().unwrap().insert(id.to_string(), token);
    }

    pub fn unregister_cancel(&self, id: &str) {
        self.cancels.lock().unwrap().remove(id);
    }

    /// Cancels or declines a transfer in any state.
    pub fn cancel(&self, id: &str) {
        if self.decide(id, false) {
            return;
        }
        if let Some(token) = self.cancels.lock().unwrap().remove(id) {
            token.cancel();
            return;
        }
        let direction = self.transfers.lock().unwrap().get(id).map(|t| t.direction);
        if direction == Some(Direction::Out) {
            self.companion.cancel_offer(self, id);
        }
        self.finish(id, Status::Cancelled, Some("Cancelled".into()));
    }

    // ------------------------------------------------------------- sending

    pub fn send_files(
        self: &Arc<Self>,
        peer_query: &str,
        items: Vec<SendItem>,
        pin: Option<String>,
    ) -> Result<String, String> {
        let peer = self
            .registry
            .get(peer_query)
            .or_else(|| self.registry.find(peer_query))
            .ok_or("Device not found")?;
        match peer.kind {
            PeerKind::LocalSend => sender::send_files(self, peer, items, pin),
            PeerKind::Web => self.companion.offer(self, &peer, items, None),
            PeerKind::QuickShare => quickshare::send_files(self, peer, items),
        }
    }

    pub fn send_text(
        self: &Arc<Self>,
        peer_query: &str,
        text: String,
        pin: Option<String>,
    ) -> Result<String, String> {
        if text.is_empty() {
            return Err("Nothing to send".into());
        }
        let peer = self
            .registry
            .get(peer_query)
            .or_else(|| self.registry.find(peer_query))
            .ok_or("Device not found")?;
        match peer.kind {
            PeerKind::LocalSend => Ok(sender::send_text(self, peer, text, pin)),
            PeerKind::Web => self.companion.offer(self, &peer, Vec::new(), Some(text)),
            PeerKind::QuickShare => Ok(quickshare::send_text(self, peer, text)),
        }
    }

    pub fn set_trusted(&self, peer_id: &str, trusted: bool) {
        // Only devices with a provable id (a certificate) can be trusted:
        // Quick Share senders and browsers are known by address or by a
        // pairing key that travels in the clear.
        if trusted && !crate::protocol::is_localsend_id(peer_id) {
            return;
        }
        self.registry.set_trusted(peer_id, trusted);
        self.update_settings(|s| {
            s.trusted.retain(|t| t != peer_id);
            if trusted {
                s.trusted.push(peer_id.to_string());
            }
        });
    }

    // -------------------------------------------------------- after receive

    pub fn on_files_received(self: &Arc<Self>, id: &str) {
        let (alias, files, auto_accepted) = {
            let transfers = self.transfers.lock().unwrap();
            let Some(t) = transfers.get(id) else { return };
            let alias = t
                .peer
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or("A device")
                .to_string();
            let files: Vec<(String, u64, Option<String>)> = t
                .files
                .iter()
                .map(|f| (f.name.clone(), f.size, f.path.clone()))
                .collect();
            (alias, files, t.auto_accepted)
        };
        tracing::info!("Received {} file(s) from {alias}", files.len());
        // From your own devices it just opens, like AirDrop: a photo, video,
        // song or PDF in its app; anything else (or several files) only shows
        // in its folder, so nothing runnable is ever started by itself.
        if auto_accepted && self.settings().open_received {
            let paths: Vec<PathBuf> = files
                .iter()
                .filter_map(|(_, _, p)| p.clone().map(PathBuf::from))
                .collect();
            match paths.as_slice() {
                [one] if desktop::safe_to_open(one) => {
                    desktop::open_path(one);
                }
                [first, ..] => {
                    let first = first.clone();
                    tokio::spawn(async move { desktop::reveal_path(&first).await });
                }
                [] => {}
            }
        }
        if !self.settings().notifications {
            return;
        }
        let (count, names) = describe_files(
            &files
                .iter()
                .map(|(n, s, _)| (n.as_str(), *s))
                .collect::<Vec<_>>(),
        );
        let first = files
            .first()
            .and_then(|(_, _, p)| p.clone())
            .map(PathBuf::from);
        // A received photo shows itself in the notification (as a thumbnail
        // we made, not the received file itself).
        let photo = files
            .iter()
            .filter(|(name, _, _)| {
                mime_guess::from_path(name).first_or_octet_stream().type_() == "image"
            })
            .find_map(|(_, _, p)| p.clone().map(PathBuf::from));
        let app = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let image = match photo {
                Some(photo) => tokio::task::spawn_blocking(move || {
                    crate::thumbnail::jpeg(&photo)
                        .and_then(|jpeg| crate::thumbnail::save_for_notification(&id, &jpeg))
                })
                .await
                .ok()
                .flatten(),
                None => None,
            };
            app.notifier
                .notify(
                    &format!("Received {count} from {alias}"),
                    &names,
                    image.as_deref(),
                    &[("default", "Open"), ("reveal", "Show in folder")],
                    move |action| {
                        let Some(path) = first else { return };
                        if action == "reveal" {
                            tokio::spawn(async move { desktop::reveal_path(&path).await });
                        } else {
                            desktop::open_path(&path);
                        }
                    },
                )
                .await;
        });
    }

    pub fn on_text_received(self: &Arc<Self>, id: &str) {
        let (alias, text, peer_id, verified) = {
            let transfers = self.transfers.lock().unwrap();
            let Some(t) = transfers.get(id) else { return };
            let alias = t
                .peer
                .get("alias")
                .and_then(Value::as_str)
                .unwrap_or("A device")
                .to_string();
            (
                alias,
                t.message.clone().unwrap_or_default(),
                t.peer_id.clone(),
                t.peer_verified,
            )
        };
        // Only your own devices, proven by their certificate, may write to
        // your clipboard or open links; anyone else's text (including the
        // browser page, whose key travels over plain HTTP) waits for "Copy".
        let own = verified && self.registry.is_trusted(&peer_id);
        let auto_copy = self.settings().copy_received_text && own;
        // A link from your own device opens in the browser right away.
        let trimmed = text.trim();
        if own
            && self.settings().open_received
            && (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
            && !trimmed.contains(char::is_whitespace)
        {
            desktop::open_url(trimmed);
        }
        let notify = self.settings().notifications;
        self.bus.emit(
            "message",
            json!({ "id": id, "from": alias, "text": text, "copied": auto_copy }),
        );
        let app = self.clone();
        tokio::spawn(async move {
            let copied = auto_copy && desktop::copy_text(&text).await;
            if !notify {
                return;
            }
            let preview: String = text.chars().take(240).collect();
            let link = text.trim();
            let link = ((link.starts_with("http://") || link.starts_with("https://"))
                && !link.contains(char::is_whitespace))
            .then(|| link.to_string());
            if let Some(link) = link {
                // Like AirDrop: a shared link opens in the browser in one click.
                let title = if copied {
                    format!("Copied link from {alias}")
                } else {
                    format!("Link from {alias}")
                };
                app.notifier
                    .notify(
                        &title,
                        &preview,
                        None,
                        &[("default", "Open"), ("open", "Open link"), ("copy", "Copy")],
                        move |action| {
                            if action == "copy" {
                                tokio::spawn(async move { desktop::copy_text(&link).await });
                            } else {
                                desktop::open_url(&link);
                            }
                        },
                    )
                    .await;
            } else if copied {
                app.notifier
                    .notify(
                        &format!("Copied text from {alias}"),
                        &preview,
                        None,
                        &[],
                        |_| {},
                    )
                    .await;
            } else {
                app.notifier
                    .notify(
                        &format!("Message from {alias}"),
                        &preview,
                        None,
                        &[("copy", "Copy")],
                        move |action| {
                            if action == "copy" {
                                tokio::spawn(async move { desktop::copy_text(&text).await });
                            }
                        },
                    )
                    .await;
            }
        });
    }

    /// Nearby Android phones show up for Quick Share (see quickshare::ble).
    pub fn wake_phones(&self) {
        if let Some(service) = self.quick_share.lock().unwrap().as_ref() {
            service.wake_phones();
        }
    }

    // --------------------------------------------------------------- window

    pub fn open_window(&self) {
        if self.ui_clients.load(Ordering::Relaxed) > 0 {
            self.bus.emit("focus", Value::Null);
            return;
        }
        if let Some(url) = self.ui_url() {
            let used = desktop::open_window(&url, &config::cache_dir().join("window-profile"));
            tracing::info!("Opened window via {used}");
        }
    }
}

/// What AirDrop would say: "a photo", "3 photos", "“Report.pdf”", "4 items".
pub fn share_phrase(files: &[(&str, &str)]) -> String {
    let all =
        |prefix: &str| !files.is_empty() && files.iter().all(|(_, mime)| mime.starts_with(prefix));
    match files {
        [] => "something".into(),
        [_] if all("image/") => "a photo".into(),
        [_] if all("video/") => "a video".into(),
        [(name, _)] => format!("“{}”", name.rsplit('/').next().unwrap_or(name)),
        _ if all("image/") => format!("{} photos", files.len()),
        _ if all("video/") => format!("{} videos", files.len()),
        _ => format!("{} items", files.len()),
    }
}

/// ("3 files", "photo.jpg, notes.pdf and 1 more · 12.4 MB")
pub fn describe_files(files: &[(&str, u64)]) -> (String, String) {
    let count = if files.len() == 1 {
        "a file".to_string()
    } else {
        format!("{} files", files.len())
    };
    let names: Vec<&str> = files.iter().take(2).map(|(n, _)| *n).collect();
    let mut text = names.join(", ");
    if files.len() > 2 {
        text.push_str(&format!(" and {} more", files.len() - 2));
    }
    let total: u64 = files.iter().map(|(_, s)| *s).sum();
    (count, format!("{text} · {}", human_size(total)))
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_phrases_read_like_airdrop() {
        assert_eq!(share_phrase(&[("a.jpg", "image/jpeg")]), "a photo");
        assert_eq!(
            share_phrase(&[("a.jpg", "image/jpeg"), ("b.png", "image/png")]),
            "2 photos"
        );
        assert_eq!(share_phrase(&[("clip.mp4", "video/mp4")]), "a video");
        assert_eq!(
            share_phrase(&[("Docs/Report.pdf", "application/pdf")]),
            "“Report.pdf”"
        );
        assert_eq!(
            share_phrase(&[("a.jpg", "image/jpeg"), ("b.pdf", "application/pdf")]),
            "2 items"
        );
    }
}
