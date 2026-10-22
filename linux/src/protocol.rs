//! LocalSend protocol v2 data model (<https://github.com/localsend/protocol>).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2.1";
pub const API: &str = "/api/localsend/v2";
pub const DEVICE_TYPES: [&str; 5] = ["mobile", "desktop", "web", "headless", "server"];
/// Longest text message we accept (LocalSend sends texts inline in `preview`).
pub const MAX_TEXT: usize = 256 * 1024;
/// Most files one request may offer.
pub const MAX_FILES: usize = 10_000;

fn default_version() -> String {
    "1.0".into()
}
fn default_port() -> u16 {
    53317
}
fn default_protocol() -> String {
    "https".into()
}
fn default_file_type() -> String {
    "application/octet-stream".into()
}

/// Accepts strings and numbers, so that slightly-off clients still parse.
fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::String(s)) => Some(s),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

/// Information a device sends about itself (register, announcements, prepare-upload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub alias: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub device_model: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub device_type: Option<String>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub download: bool,
}

impl DeviceInfo {
    /// Normalizes untrusted input: bounded lengths, known enum values.
    pub fn sanitized(mut self) -> Result<Self, &'static str> {
        self.alias = truncate(&strip_invisible(self.alias.trim()), 128);
        if self.alias.is_empty() {
            return Err("alias is required");
        }
        self.version = truncate(&self.version, 16);
        self.device_model = self
            .device_model
            .map(|m| truncate(&strip_invisible(&m), 128));
        let device_type = self.device_type.unwrap_or_else(|| "desktop".into());
        self.device_type = Some(if DEVICE_TYPES.contains(&device_type.as_str()) {
            device_type
        } else {
            "desktop".into()
        });
        if self.protocol != "http" {
            self.protocol = "https".into();
        }
        self.fingerprint = truncate(&self.fingerprint, 128);
        if self.port == 0 {
            self.port = default_port();
        }
        Ok(self)
    }

    pub fn is_https(&self) -> bool {
        self.protocol == "https"
    }

    pub fn register_json(&self) -> Value {
        serde_json::to_value(self).expect("serializable")
    }

    pub fn announcement_json(&self, announce: bool) -> Value {
        let mut value = self.register_json();
        value["announce"] = json!(announce);
        // Protocol v1 clients look for "announcement".
        value["announcement"] = json!(announce);
        value
    }

    /// Response body of /register and /info (no port/protocol).
    pub fn info_json(&self) -> Value {
        json!({
            "alias": self.alias,
            "version": self.version,
            "deviceModel": self.device_model,
            "deviceType": self.device_type,
            "fingerprint": self.fingerprint,
            "download": self.download,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct MulticastMessage {
    #[serde(flatten)]
    pub info: DeviceInfo,
    #[serde(default)]
    pub announce: Option<bool>,
    #[serde(default)]
    pub announcement: Option<bool>,
}

impl MulticastMessage {
    pub fn is_announcement(&self) -> bool {
        self.announce.or(self.announcement).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDto {
    pub id: String,
    pub file_name: String,
    pub size: u64,
    #[serde(default = "default_file_type")]
    pub file_type: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

impl FileDto {
    pub fn sanitized(mut self) -> Result<Self, &'static str> {
        if self.id.is_empty() || self.file_name.is_empty() {
            return Err("file id and fileName are required");
        }
        self.id = truncate(&self.id, 128);
        self.file_name = truncate(&self.file_name, 1024);
        // Text arrives inline; anything else may carry a small image preview.
        let text = self.file_type.starts_with("text") && self.preview.is_some();
        if text && self.preview.as_ref().is_some_and(|p| p.len() > MAX_TEXT) {
            return Err("text message too long");
        }
        if !text
            && self
                .preview
                .as_ref()
                .is_some_and(|p| p.len() > crate::thumbnail::MAX_DATA_URL)
        {
            self.preview = None;
        }
        if !self
            .sha256
            .as_ref()
            .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            self.sha256 = None;
        }
        // Protocol v1 used short names instead of MIME types.
        self.file_type = match self.file_type.as_str() {
            "image" => "image/*".into(),
            "video" => "video/*".into(),
            "pdf" => "application/pdf".into(),
            "text" => "text/plain".into(),
            "apk" => "application/vnd.android.package-archive".into(),
            "other" | "" => default_file_type(),
            other => truncate(other, 128),
        };
        Ok(self)
    }

    /// LocalSend sends short texts as a single text "file" whose content is in `preview`.
    pub fn is_text_message(&self) -> bool {
        self.file_type.starts_with("text/") && self.preview.is_some()
    }

    pub fn for_path(path: &Path, name: &str) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let iso = |t: std::io::Result<std::time::SystemTime>| {
            t.ok().map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string()
            })
        };
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            file_name: name.to_string(),
            size: meta.len(),
            file_type: mime_guess::from_path(path)
                .first_or_octet_stream()
                .essence_str()
                .to_string(),
            sha256: None,
            preview: None,
            metadata: Some(FileMetadata {
                modified: iso(meta.modified()),
                accessed: iso(meta.accessed()),
            }),
        })
    }

    pub fn for_text(text: &str) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        Self {
            file_name: format!("{id}.txt"),
            id,
            size: text.len() as u64,
            file_type: "text/plain".into(),
            sha256: None,
            preview: Some(text.to_string()),
            metadata: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PrepareUploadRequest {
    pub info: DeviceInfo,
    pub files: HashMap<String, FileDto>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    pub files: HashMap<String, String>,
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Characters that change how text is displayed without being visible
/// (bidi overrides and isolates, zero-width marks): "photo\u{202E}gpj.sh"
/// would otherwise read as "photohs.jpg".
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

fn strip_invisible(s: &str) -> String {
    s.chars().filter(|&c| !is_invisible(c)).collect()
}

/// A LocalSend device id: a certificate fingerprint (64 hex digits) or, for
/// plain-HTTP peers, a random token. Ids of other kinds of peers ("web-…",
/// "quickshare:…") cannot be claimed over LocalSend.
pub fn is_localsend_id(id: &str) -> bool {
    (8..=128).contains(&id.len())
        && !id.starts_with("web-")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Cuts `s` to at most `max` bytes on a character boundary.
fn truncate_bytes(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Longest file or folder name we create, in bytes (Linux allows 255; the
/// partial download adds a short suffix).
const MAX_NAME_BYTES: usize = 200;
/// Deepest folder structure a sender may create.
const MAX_DEPTH: usize = 16;

/// Turns a sender-supplied name (which may contain folders) into a safe relative path.
pub fn safe_relative_path(file_name: &str) -> PathBuf {
    let normalized = file_name.replace('\\', "/");
    let mut parts: Vec<String> = Vec::new();
    for component in Path::new(&normalized).components() {
        if let Component::Normal(part) = component {
            let part = part.to_string_lossy();
            let cleaned: String = part
                .chars()
                .filter(|&c| !is_invisible(c))
                .map(|c| if c.is_control() { '_' } else { c })
                .collect();
            let cleaned = cleaned.trim().trim_start_matches(['.', ' ']);
            let cleaned = truncate_bytes(cleaned, MAX_NAME_BYTES).trim_end();
            parts.push(if cleaned.is_empty() {
                "_".to_string()
            } else {
                cleaned.to_string()
            });
        }
    }
    if parts.len() > MAX_DEPTH {
        // Keep the top folders and the file's own name.
        let name = parts.pop().unwrap_or_default();
        parts.truncate(MAX_DEPTH - 1);
        parts.push(name);
    }
    let mut out: PathBuf = parts.into_iter().collect();
    if out.as_os_str().is_empty() {
        out.push("file");
    }
    out
}

pub fn parse_iso(value: Option<&str>) -> Option<std::time::SystemTime> {
    let value = value?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_localsend_register() {
        let info: DeviceInfo = serde_json::from_str(
            r#"{"alias":"Secret Banana","version":"2.0","deviceModel":"Windows","deviceType":"fridge",
                "fingerprint":"abc","port":53317,"protocol":"https","download":true}"#,
        )
        .unwrap();
        let info = info.sanitized().unwrap();
        assert_eq!(info.device_type.as_deref(), Some("desktop"));
        assert!(info.download);
    }

    #[test]
    fn parses_multicast_with_defaults() {
        let msg: MulticastMessage = serde_json::from_str(
            r#"{"alias":"Nice Orange","fingerprint":"x","announcement":true}"#,
        )
        .unwrap();
        assert!(msg.is_announcement());
        assert_eq!(msg.info.port, 53317);
        assert_eq!(msg.info.protocol, "https");
    }

    #[test]
    fn text_messages_are_detected() {
        let dto = FileDto::for_text("hello");
        assert!(dto.is_text_message());
        let parsed: FileDto = serde_json::from_value(serde_json::to_value(&dto).unwrap()).unwrap();
        assert_eq!(parsed.preview.as_deref(), Some("hello"));
    }

    #[test]
    fn unsafe_names_are_neutralized() {
        assert_eq!(
            safe_relative_path("../../etc/passwd"),
            PathBuf::from("etc/passwd")
        );
        assert_eq!(
            safe_relative_path("/abs/file.txt"),
            PathBuf::from("abs/file.txt")
        );
        assert_eq!(
            safe_relative_path("..\\win\\x.exe"),
            PathBuf::from("win/x.exe")
        );
        assert_eq!(safe_relative_path(".bashrc"), PathBuf::from("bashrc"));
        assert_eq!(safe_relative_path(""), PathBuf::from("file"));
        assert_eq!(
            safe_relative_path("Album/photo.jpg"),
            PathBuf::from("Album/photo.jpg")
        );
        // Right-to-left override: would display as "photohs.jpg".
        assert_eq!(
            safe_relative_path("photo\u{202E}gpj.sh"),
            PathBuf::from("photogpj.sh")
        );
        // Long multi-byte names fit the file system; deep trees are flattened.
        let long = safe_relative_path(&"é".repeat(300));
        assert!(long.as_os_str().len() <= 200);
        let deep = safe_relative_path(&format!("{}x.txt", "d/".repeat(40)));
        assert_eq!(deep.components().count(), 16);
        assert!(deep.ends_with("x.txt"));
    }

    #[test]
    fn hostile_requests_are_bounded() {
        let dto = |file_type: &str, preview: &str| FileDto {
            id: "1".into(),
            file_name: "a".into(),
            size: 1,
            file_type: file_type.into(),
            sha256: Some("not-a-hash".into()),
            preview: Some(preview.into()),
            metadata: None,
        };
        assert!(dto("text/plain", &"x".repeat(MAX_TEXT + 1))
            .sanitized()
            .is_err());
        let file = dto("image/jpeg", &"x".repeat(400 * 1024))
            .sanitized()
            .unwrap();
        assert_eq!((file.preview, file.sha256), (None, None));
        assert!(is_localsend_id(&"AB12".repeat(16)));
        assert!(!is_localsend_id("quickshare:ABCD"));
        assert!(!is_localsend_id("web-0123456789"));
        assert!(!is_localsend_id("short"));
    }
}
