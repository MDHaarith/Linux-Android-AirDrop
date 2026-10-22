//! Settings and filesystem locations.

use std::path::{Path, PathBuf};

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_PORT: u16 = 53317;
pub const DEFAULT_MULTICAST_GROUP: &str = "224.0.0.167";
pub const DEFAULT_WEB_PORT: u16 = 53318;
/// Quick Share listens here (falls back to any free port).
pub const QUICK_SHARE_PORT: u16 = 53319;
const APP_ID: &str = "airbridge";

const ADJECTIVES: &[&str] = &[
    "Swift", "Bright", "Calm", "Clever", "Cosmic", "Crisp", "Gentle", "Golden", "Happy", "Lucky",
    "Mellow", "Misty", "Nimble", "Quiet", "Silver", "Sunny", "Velvet", "Vivid", "Witty", "Zesty",
];
const NOUNS: &[&str] = &[
    "Falcon", "Otter", "Comet", "Maple", "Panda", "Lynx", "Nebula", "Pebble", "Koala", "Orbit",
    "Willow", "Harbor", "Meadow", "Aurora", "Badger", "Cedar", "Dolphin", "Ember", "Fox",
    "Juniper",
];
const GENERIC_HOSTNAMES: &[&str] = &[
    "localhost",
    "fedora",
    "ubuntu",
    "debian",
    "archlinux",
    "manjaro",
    "linux",
    "pop-os",
    "kali",
    "raspberrypi",
    "endeavouros",
    "nixos",
];

pub fn random_alias() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "{} {}",
        ADJECTIVES.choose(&mut rng).unwrap(),
        NOUNS.choose(&mut rng).unwrap()
    )
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn default_alias() -> String {
    let host = hostname();
    let host = host.split('.').next().unwrap_or_default();
    if host.is_empty() || GENERIC_HOSTNAMES.contains(&host.to_lowercase().as_str()) {
        return random_alias();
    }
    // "mohamed-thinkpad" -> "Mohamed Thinkpad"
    host.split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn default_device_model() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|l| {
                l.strip_prefix("NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Linux".into())
}

fn base_override() -> Option<PathBuf> {
    std::env::var_os("AIRBRIDGE_HOME").map(PathBuf::from)
}

pub fn config_dir() -> PathBuf {
    base_override()
        .map(|b| b.join("config"))
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| home().join(".config"))
                .join(APP_ID)
        })
}

pub fn data_dir() -> PathBuf {
    base_override().map(|b| b.join("data")).unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| home().join(".local/share"))
            .join(APP_ID)
    })
}

pub fn cache_dir() -> PathBuf {
    base_override().map(|b| b.join("cache")).unwrap_or_else(|| {
        dirs::cache_dir()
            .unwrap_or_else(|| home().join(".cache"))
            .join(APP_ID)
    })
}

pub fn runtime_dir() -> PathBuf {
    if let Some(base) = base_override() {
        return base.join("run");
    }
    match dirs::runtime_dir() {
        Some(dir) => dir.join(APP_ID),
        None => PathBuf::from(format!("/tmp/{APP_ID}-{}", current_uid())),
    }
}

fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and cannot fail.
    unsafe { nix::libc::getuid() }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn default_download_dir() -> PathBuf {
    if let Some(base) = base_override() {
        return base.join("downloads");
    }
    dirs::download_dir().unwrap_or_else(|| home().join("Downloads"))
}

fn random_key() -> String {
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(20)
        .map(char::from)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub alias: String,
    pub device_model: String,
    pub port: u16,
    pub multicast_port: u16,
    pub multicast_group: String,
    pub https: bool,
    pub download_dir: PathBuf,
    /// Fingerprints of devices the user trusts.
    pub trusted: Vec<String>,
    /// Accept transfers from trusted devices without asking.
    pub auto_accept_trusted: bool,
    /// Optional PIN senders must provide (LocalSend `?pin=`).
    pub pin: String,
    /// Copy received text messages to the clipboard.
    pub copy_received_text: bool,
    /// Phones without an app can scan a QR code and share through the browser.
    pub web_companion: bool,
    pub web_port: u16,
    /// Secret contained in the companion QR code.
    pub web_key: String,
    /// Port of the local (127.0.0.1 only) desktop UI. 0 picks a free port.
    pub ui_port: u16,
    pub notifications: bool,
    pub theme: String,
    /// Be visible to Android's built-in Quick Share (no app needed).
    pub quick_share: bool,
    /// Drop zone on the screen edge: drag anything there to send it.
    pub edge_drop: bool,
    /// "right" or "left".
    pub edge_side: String,
    /// Device id, or "@all" when the user explicitly chooses every device.
    pub drop_recipient: Option<String>,
    /// Open what your own (trusted) devices send right away, like AirDrop
    /// between your own Apple devices.
    pub open_received: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alias: default_alias(),
            device_model: default_device_model(),
            port: DEFAULT_PORT,
            multicast_port: DEFAULT_PORT,
            multicast_group: DEFAULT_MULTICAST_GROUP.into(),
            https: true,
            download_dir: default_download_dir(),
            trusted: Vec::new(),
            auto_accept_trusted: true,
            pin: String::new(),
            copy_received_text: true,
            // Plain HTTP on the local network: off until you pair a phone.
            web_companion: false,
            web_port: DEFAULT_WEB_PORT,
            web_key: random_key(),
            ui_port: 0,
            notifications: true,
            theme: "auto".into(),
            quick_share: true,
            edge_drop: true,
            edge_side: "right".into(),
            drop_recipient: None,
            open_received: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(settings) => {
                    // Persist defaults for missing keys (e.g. the pairing key)
                    // so they stay stable across restarts.
                    if serde_json::to_string_pretty(&settings).ok().as_deref()
                        != Some(text.as_str())
                    {
                        if let Err(err) = settings.save() {
                            tracing::warn!("Could not save settings: {err}");
                        }
                    }
                    return settings;
                }
                Err(err) => tracing::warn!("{}: {err}; using defaults", path.display()),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!("{}: {err}; using defaults", path.display()),
        }
        let settings = Self::default();
        if let Err(err) = settings.save() {
            tracing::warn!("Could not save settings: {err}");
        }
        settings
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        write_private(&path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    /// The settings the UI may see (without the companion secret).
    pub fn public_json(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("serializable");
        if let Some(map) = value.as_object_mut() {
            map.remove("webKey");
        }
        value
    }

    /// Applies edits coming from the UI; returns the names of changed keys.
    pub fn apply(&mut self, changes: &Value) -> Vec<&'static str> {
        let mut changed = Vec::new();
        let Some(map) = changes.as_object() else {
            return changed;
        };
        macro_rules! set_bool {
            ($key:literal, $field:ident) => {
                if let Some(v) = map.get($key).and_then(Value::as_bool) {
                    if self.$field != v {
                        self.$field = v;
                        changed.push($key);
                    }
                }
            };
        }
        if let Some(alias) = map.get("alias").and_then(Value::as_str) {
            let alias: String = alias.trim().chars().take(64).collect();
            let alias = if alias.is_empty() {
                default_alias()
            } else {
                alias
            };
            if self.alias != alias {
                self.alias = alias;
                changed.push("alias");
            }
        }
        if let Some(dir) = map.get("downloadDir").and_then(Value::as_str) {
            if let Err(err) = check_download_dir(Path::new(dir)) {
                tracing::warn!("Ignoring download folder {dir}: {err}");
                return changed;
            }
            let dir = PathBuf::from(dir);
            if dir.is_absolute() && self.download_dir != dir {
                self.download_dir = dir;
                changed.push("downloadDir");
            }
        }
        if let Some(pin) = map.get("pin").and_then(Value::as_str) {
            let pin: String = pin
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(32)
                .collect();
            if self.pin != pin && (pin.is_empty() || pin.len() >= MIN_PIN_LEN) {
                self.pin = pin;
                changed.push("pin");
            }
        }
        if let Some(theme) = map.get("theme").and_then(Value::as_str) {
            if ["auto", "dark", "light"].contains(&theme) && self.theme != theme {
                self.theme = theme.into();
                changed.push("theme");
            }
        }
        set_bool!("autoAcceptTrusted", auto_accept_trusted);
        set_bool!("copyReceivedText", copy_received_text);
        set_bool!("webCompanion", web_companion);
        set_bool!("notifications", notifications);
        set_bool!("quickShare", quick_share);
        set_bool!("edgeDrop", edge_drop);
        set_bool!("openReceived", open_received);
        if let Some(value) = map.get("dropRecipient") {
            if value.is_null()
                || value
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 256)
            {
                let recipient = value.as_str().map(str::to_string);
                if self.drop_recipient != recipient {
                    self.drop_recipient = recipient;
                    changed.push("dropRecipient");
                }
            }
        }
        if let Some(side) = map.get("edgeSide").and_then(Value::as_str) {
            if ["left", "right"].contains(&side) && self.edge_side != side {
                self.edge_side = side.into();
                changed.push("edgeSide");
            }
        }
        changed
    }

    pub fn regenerate_web_key(&mut self) {
        self.web_key = random_key();
    }
}

/// Writes a file readable only by the current user, atomically.
/// Shortest PIN we accept: short PINs fall to guessing.
pub const MIN_PIN_LEN: usize = 6;

/// Settings changes the window may not make, with the reason.
pub fn check_changes(map: &Value) -> Result<(), String> {
    if let Some(pin) = map.get("pin").and_then(Value::as_str) {
        let len = pin.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        if len > 0 && len < MIN_PIN_LEN {
            return Err(format!("A PIN needs at least {MIN_PIN_LEN} characters"));
        }
    }
    if let Some(dir) = map.get("downloadDir").and_then(Value::as_str) {
        check_download_dir(Path::new(dir))?;
    }
    Ok(())
}

/// Received files must not land where they would be run or trusted
/// automatically (autostart, shell and SSH configuration, …): no hidden
/// folders in the home folder, no system folders.
fn check_download_dir(dir: &Path) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err("Choose a full folder path".into());
    }
    let home = home();
    let inside_home = dir.strip_prefix(&home).ok();
    let hidden = inside_home.is_some_and(|rest| {
        rest.components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    });
    let system = inside_home.is_none()
        && [
            "/etc", "/usr", "/bin", "/sbin", "/lib", "/boot", "/root", "/var", "/proc", "/sys",
        ]
        .iter()
        .any(|p| dir.starts_with(p));
    if hidden || system {
        return Err("Choose a regular folder, not a hidden or system one".into());
    }
    Ok(())
}

pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    // The folder must be ours and not writable by others: a folder another
    // user prepared (say under /tmp) could swap or read our files.
    let meta = std::fs::symlink_metadata(parent)?;
    if !meta.is_dir() || meta.uid() != current_uid() || meta.mode() & 0o022 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is not a private folder", parent.display()),
        ));
    }
    // A fresh temporary file: never an existing one, never through a symlink.
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&tmp)?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_recipient_round_trips_and_can_be_cleared() {
        let mut settings = Settings::default();
        settings.apply(&serde_json::json!({"dropRecipient": "phone"}));
        let json = serde_json::to_string(&settings).unwrap();
        let saved: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(saved.drop_recipient.as_deref(), Some("phone"));
        settings.apply(&serde_json::json!({"dropRecipient": "@all"}));
        assert_eq!(settings.public_json()["dropRecipient"], "@all");
        settings.apply(&serde_json::json!({"dropRecipient": null}));
        assert!(settings.drop_recipient.is_none());
        settings.apply(&serde_json::json!({"dropRecipient": 42}));
        assert!(settings.drop_recipient.is_none());
    }
}
