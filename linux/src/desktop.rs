//! Desktop integration: notifications, clipboard, file manager, file picker and the app window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use zbus::zvariant::Value;

// ------------------------------------------------------------- notifications

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Desktop notifications over D-Bus (org.freedesktop.Notifications).
#[derive(Default)]
pub struct Notifier {
    proxy: tokio::sync::OnceCell<Option<NotificationsProxy<'static>>>,
    /// The server renders markup in the body (then text must be escaped).
    markup: tokio::sync::OnceCell<bool>,
}

/// Text from other devices (names, messages) as plain notification text:
/// markup escaped where the server renders it, so a hostile name cannot
/// add links or hide what is being offered, and no invisible bidi controls.
fn plain_text(text: &str, markup: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}' => {}
            '&' if markup => out.push_str("&amp;"),
            '<' if markup => out.push_str("&lt;"),
            '>' if markup => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Closes a question notification once it is no longer relevant.
struct CloseOnDrop {
    proxy: NotificationsProxy<'static>,
    id: u32,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let (proxy, id) = (self.proxy.clone(), self.id);
        tokio::spawn(async move {
            let _ = proxy.close_notification(id).await;
        });
    }
}

impl Notifier {
    async fn proxy(&self) -> Option<NotificationsProxy<'static>> {
        self.proxy
            .get_or_init(|| async {
                let connection = match zbus::Connection::session().await {
                    Ok(c) => c,
                    Err(err) => {
                        tracing::info!("No D-Bus session bus, notifications disabled ({err})");
                        return None;
                    }
                };
                NotificationsProxy::new(&connection).await.ok()
            })
            .await
            .clone()
    }

    pub async fn available(&self) -> bool {
        self.proxy().await.is_some()
    }

    async fn show(
        &self,
        proxy: &NotificationsProxy<'static>,
        summary: &str,
        body: &str,
        image: Option<&Path>,
        actions: &[(&str, &str)],
        critical: bool,
    ) -> Option<u32> {
        let markup = *self
            .markup
            .get_or_init(|| async {
                proxy
                    .get_capabilities()
                    .await
                    .is_ok_and(|caps| caps.iter().any(|c| c == "body-markup"))
            })
            .await;
        let summary = &plain_text(summary, false);
        let body = &plain_text(body, markup);
        let mut flat = Vec::new();
        for (key, label) in actions {
            flat.push(*key);
            flat.push(*label);
        }
        let mut hints = HashMap::new();
        hints.insert("desktop-entry", Value::from("airbridge"));
        hints.insert("urgency", Value::U8(if critical { 2 } else { 1 }));
        let image = image.map(|p| p.to_string_lossy().into_owned());
        if let Some(path) = image.as_deref() {
            // Plasma and GNOME show this picture in the notification.
            hints.insert("image-path", Value::from(path));
        }
        let timeout = if critical { 0 } else { -1 };
        match proxy
            .notify(
                "AirBridge",
                0,
                "airbridge",
                summary,
                body,
                &flat,
                hints,
                timeout,
            )
            .await
        {
            Ok(id) => Some(id),
            Err(err) => {
                tracing::debug!("notification failed: {err}");
                None
            }
        }
    }

    /// Fire-and-forget notification; `on_action` runs for a clicked button.
    pub async fn notify<F>(
        &self,
        summary: &str,
        body: &str,
        image: Option<&Path>,
        actions: &[(&str, &str)],
        on_action: F,
    ) where
        F: FnOnce(String) + Send + 'static,
    {
        let Some(proxy) = self.proxy().await else {
            return;
        };
        let mut stream = match proxy.receive_action_invoked().await {
            Ok(s) => s,
            Err(_) => return,
        };
        let Some(id) = self
            .show(&proxy, summary, body, image, actions, false)
            .await
        else {
            return;
        };
        if actions.is_empty() {
            return;
        }
        tokio::spawn(async move {
            let wait = async {
                while let Some(signal) = stream.next().await {
                    if let Ok(args) = signal.args() {
                        if args.id == id {
                            return Some(args.action_key);
                        }
                    }
                }
                None
            };
            if let Ok(Some(action)) = tokio::time::timeout(Duration::from_secs(600), wait).await {
                on_action(action);
            }
        });
    }

    /// Shows a notification with buttons and waits for the answer. Dropping the
    /// future (e.g. because the user answered in the window) closes it.
    pub async fn ask(
        &self,
        summary: &str,
        body: &str,
        image: Option<&Path>,
        actions: &[(&str, &str)],
    ) -> Option<String> {
        let proxy = self.proxy().await?;
        let mut invoked = proxy.receive_action_invoked().await.ok()?;
        let mut closed = proxy.receive_notification_closed().await.ok()?;
        let id = self
            .show(&proxy, summary, body, image, actions, true)
            .await?;
        let _guard = CloseOnDrop {
            proxy: proxy.clone(),
            id,
        };
        loop {
            tokio::select! {
                Some(signal) = invoked.next() => {
                    if let Ok(args) = signal.args() {
                        if args.id == id {
                            return Some(args.action_key);
                        }
                    }
                }
                Some(signal) = closed.next() => {
                    if let Ok(args) = signal.args() {
                        if args.id == id {
                            return None;
                        }
                    }
                }
                else => return None,
            }
        }
    }
}

// ----------------------------------------------------------------- clipboard

fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(binary))
            .find(|p| p.is_file())
    })
}

fn clipboard_commands(copy: bool) -> Vec<Vec<&'static str>> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut candidates: Vec<Vec<&'static str>> = Vec::new();
    if copy {
        if wayland {
            candidates.push(vec!["wl-copy"]);
        }
        candidates.push(vec!["xclip", "-selection", "clipboard"]);
        candidates.push(vec!["xsel", "--clipboard", "--input"]);
    } else {
        if wayland {
            candidates.push(vec!["wl-paste", "--no-newline"]);
        }
        candidates.push(vec!["xclip", "-selection", "clipboard", "-o"]);
        candidates.push(vec!["xsel", "--clipboard", "--output"]);
    }
    candidates
        .into_iter()
        .filter(|c| which(c[0]).is_some())
        .collect()
}

pub async fn copy_text(text: &str) -> bool {
    for cmd in clipboard_commands(true) {
        // These tools fork to keep serving the selection: stdout must not be a pipe.
        let Ok(mut child) = Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes()).await;
        }
        if let Ok(Ok(status)) = tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            if status.success() {
                return true;
            }
        }
    }
    false
}

pub async fn read_clipboard() -> Option<String> {
    for cmd in clipboard_commands(false) {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new(cmd[0])
                .args(&cmd[1..])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = output {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
    }
    None
}

// ------------------------------------------------------------- files/folders

fn spawn_detached(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

/// Opens a web link in the default browser.
pub fn open_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && spawn_detached("xdg-open", &[url])
}

/// Kinds of files that open in a viewer or player by themselves without
/// risk: photos, videos, music, PDFs and plain text. Decided from the name we
/// saved it under, never from the type the sender claimed. Scriptable or
/// runnable formats (HTML, SVG, scripts, .desktop, AppImage, …) are not.
pub fn safe_to_open(path: &Path) -> bool {
    const VIEWABLE: [&str; 27] = [
        "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "avif", "bmp", "tif", "tiff", "mp4",
        "m4v", "mov", "webm", "mkv", "avi", "mp3", "m4a", "aac", "flac", "ogg", "opus", "wav",
        "pdf", "txt", "md",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIEWABLE.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn open_path(path: &Path) -> bool {
    path.exists() && spawn_detached("xdg-open", &[&path.to_string_lossy()])
}

fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Shows a file highlighted in the file manager, falling back to opening its folder.
pub async fn reveal_path(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    if let Ok(connection) = zbus::Connection::session().await {
        let uri = file_uri(path);
        let shown = connection
            .call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .await;
        if shown.is_ok() {
            return true;
        }
    }
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    open_path(folder)
}

/// Native file chooser (xdg-desktop-portal, falling back to zenity).
pub async fn pick(directory: bool) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    if directory {
        rfd::AsyncFileDialog::new()
            .set_title("Choose a folder")
            .set_directory(&home)
            .pick_folder()
            .await
            .map(|f| vec![f.path().to_path_buf()])
            .unwrap_or_default()
    } else {
        rfd::AsyncFileDialog::new()
            .set_title("Choose files to send")
            .set_directory(&home)
            .pick_files()
            .await
            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
            .unwrap_or_default()
    }
}

// -------------------------------------------------------------------- window

const BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "brave-browser",
    "brave",
    "microsoft-edge",
    "microsoft-edge-stable",
    "vivaldi",
];

/// Opens the UI in the nicest available way; returns what was used.
pub fn open_window(url: &str, profile_dir: &Path) -> &'static str {
    // 1. Our own native window (built with the `webview` feature).
    #[cfg(feature = "webview")]
    if let Ok(exe) = std::env::current_exe() {
        if std::process::Command::new(exe)
            .args(["window", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return "webview";
        }
    }
    // 2. A chromeless Chromium-family app window.
    for name in BROWSERS {
        if which(name).is_some() {
            let _ = std::fs::create_dir_all(profile_dir);
            let app = format!("--app={url}");
            let profile = format!("--user-data-dir={}", profile_dir.display());
            if spawn_detached(
                name,
                &[
                    &app,
                    "--class=AirBridge",
                    &profile,
                    "--window-size=1120,780",
                    "--no-first-run",
                    "--no-default-browser-check",
                ],
            ) {
                return name;
            }
        }
    }
    // 3. Whatever browser the desktop prefers.
    if spawn_detached("xdg-open", &[url]) {
        return "xdg-open";
    }
    tracing::warn!("Open {url} in a browser to use AirBridge");
    "none"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_text_cannot_carry_markup_or_bidi_tricks() {
        assert_eq!(
            plain_text("<a href=\"x\">Invoice</a> & co", true),
            "&lt;a href=\"x\"&gt;Invoice&lt;/a&gt; &amp; co"
        );
        assert_eq!(plain_text("a & b", false), "a & b");
        assert_eq!(plain_text("photo\u{202E}gpj.sh", false), "photogpj.sh");
    }

    #[test]
    fn only_viewable_files_open_by_themselves() {
        assert!(safe_to_open(Path::new("/d/IMG_1.JPG")));
        assert!(safe_to_open(Path::new("/d/report.pdf")));
        for risky in [
            "x.desktop",
            "run.sh",
            "app.AppImage",
            "page.html",
            "logo.svg",
            "noext",
        ] {
            assert!(!safe_to_open(&Path::new("/d").join(risky)), "{risky}");
        }
    }
}
