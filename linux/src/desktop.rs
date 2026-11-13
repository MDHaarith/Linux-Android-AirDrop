//! Desktop integration: notifications, clipboard, file manager, file picker and the app window.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

#[cfg(target_os = "linux")]
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
#[cfg(target_os = "linux")]
use zbus::zvariant::Value;

// ------------------------------------------------------------- notifications

#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
struct CloseOnDrop {
    proxy: NotificationsProxy<'static>,
    id: u32,
}

#[cfg(target_os = "linux")]
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let (proxy, id) = (self.proxy.clone(), self.id);
        tokio::spawn(async move {
            let _ = proxy.close_notification(id).await;
        });
    }
}

#[cfg(target_os = "linux")]
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

/// Windows toast notifications. Buttons answer while AirBridge runs (the
/// toast reports back to this process); no COM activator is registered.
#[cfg(windows)]
#[derive(Default)]
pub struct Notifier {
    registered: std::sync::OnceLock<bool>,
}

#[cfg(windows)]
mod toast {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use tokio::sync::oneshot;
    use windows::core::{IInspectable, Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
        ToastNotificationManager, ToastNotifier,
    };

    const APP_ID: &str = "io.github.mdhaarith.AirBridge";

    /// Names this app (and its icon) for toasts, as the registry entry
    /// unpackaged apps use instead of a Start menu shortcut.
    pub fn register() -> anyhow::Result<()> {
        use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
        let icon = crate::config::data_dir().join("airbridge.png");
        std::fs::create_dir_all(crate::config::data_dir())?;
        std::fs::write(&icon, include_bytes!("../packaging/windows/airbridge.png"))?;
        let key = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
        for (name, value) in [
            ("DisplayName", "AirBridge".to_string()),
            ("IconUri", icon.to_string_lossy().into_owned()),
        ] {
            let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
            // SAFETY: `data` is a NUL-terminated UTF-16 string of the given byte length.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &key,
                    &HSTRING::from(name),
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            }
            .ok()?;
        }
        Ok(())
    }

    fn notifier() -> windows::core::Result<ToastNotifier> {
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
    }

    /// A shown toast; the receiver gets the action key ("default" for the
    /// toast itself), or None when it was dismissed.
    pub struct Shown {
        pub toast: ToastNotification,
        pub answer: oneshot::Receiver<Option<String>>,
    }

    pub fn show(
        summary: &str,
        body: &str,
        image: Option<&Path>,
        actions: &[(&str, &str)],
        question: bool,
    ) -> anyhow::Result<Shown> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(super::toast_xml(
            summary, body, image, actions, question,
        )))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;

        let (tx, answer) = oneshot::channel();
        let tx = Arc::new(Mutex::new(Some(tx)));
        let reply = move |tx: &Mutex<Option<oneshot::Sender<Option<String>>>>, value| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(value);
            }
        };
        let (activated, dismissed, failed) = (tx.clone(), tx.clone(), tx);
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, args| {
                let key = args
                    .as_ref()
                    .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
                    .and_then(|args| args.Arguments().ok())
                    .map(|key| key.to_string())
                    .unwrap_or_else(|| "default".into());
                reply(&activated, Some(key));
                Ok(())
            },
        ))?;
        toast.Dismissed(&TypedEventHandler::<
            ToastNotification,
            ToastDismissedEventArgs,
        >::new(move |_, args| {
            let reason = args.as_ref().and_then(|args| args.Reason().ok());
            // Timed out: it waits in the notification center, buttons still work.
            if reason != Some(ToastDismissalReason::TimedOut) {
                reply(&dismissed, None);
            }
            Ok(())
        }))?;
        toast.Failed(&TypedEventHandler::new(move |_, _| {
            reply(&failed, None);
            Ok(())
        }))?;
        notifier()?.Show(&toast)?;
        Ok(Shown { toast, answer })
    }

    pub fn hide(toast: &ToastNotification) {
        if let Ok(notifier) = notifier() {
            let _ = notifier.Hide(toast);
        }
    }
}

/// The toast: text from other devices escaped, so a name cannot add
/// buttons, images or markup. "default" is a click on the toast itself.
#[cfg(any(windows, test))]
fn toast_xml(
    summary: &str,
    body: &str,
    image: Option<&Path>,
    actions: &[(&str, &str)],
    question: bool,
) -> String {
    let xml = |text: &str| plain_text(text, true).replace('"', "&quot;");
    let mut doc = format!(
        "<toast launch=\"default\"{}><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text>",
        // A question stays on screen until it is answered.
        if question { " scenario=\"reminder\"" } else { "" },
        xml(summary),
        xml(body)
    );
    if let Some(image) = image {
        doc += &format!("<image src=\"{}\"/>", xml(&image.to_string_lossy()));
    }
    doc += "</binding></visual><actions>";
    for (key, label) in actions.iter().filter(|(key, _)| *key != "default") {
        doc += &format!(
            "<action content=\"{}\" arguments=\"{}\" activationType=\"foreground\"/>",
            xml(label),
            xml(key)
        );
    }
    doc + "</actions></toast>"
}

#[cfg(windows)]
impl Notifier {
    pub async fn available(&self) -> bool {
        *self.registered.get_or_init(|| {
            toast::register()
                .inspect_err(|err| tracing::info!("Notifications unavailable: {err:#}"))
                .is_ok()
        })
    }

    /// Shows a toast with buttons and waits for the answer. Dropping the
    /// future (e.g. because the user answered in the window) removes it.
    pub async fn ask(
        &self,
        summary: &str,
        body: &str,
        image: Option<&Path>,
        actions: &[(&str, &str)],
    ) -> Option<String> {
        if !self.available().await {
            return None;
        }
        let shown = toast::show(summary, body, image, actions, true)
            .inspect_err(|err| tracing::debug!("notification failed: {err:#}"))
            .ok()?;
        struct Hide(windows::UI::Notifications::ToastNotification);
        impl Drop for Hide {
            fn drop(&mut self) {
                toast::hide(&self.0);
            }
        }
        let _hide = Hide(shown.toast);
        shown.answer.await.ok().flatten()
    }

    /// Fire-and-forget toast; `on_action` runs for a clicked button or the toast.
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
        if !self.available().await {
            return;
        }
        match toast::show(summary, body, image, actions, false) {
            Ok(shown) => {
                tokio::spawn(async move {
                    let _toast = shown.toast;
                    if let Ok(Ok(Some(action))) =
                        tokio::time::timeout(Duration::from_secs(600), shown.answer).await
                    {
                        on_action(action);
                    }
                });
            }
            Err(err) => tracing::debug!("notification failed: {err:#}"),
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

#[cfg(target_os = "linux")]
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

#[cfg(windows)]
fn clipboard_commands(copy: bool) -> Vec<Vec<&'static str>> {
    let script = if copy {
        "[Console]::InputEncoding = [Text.UTF8Encoding]::new(); Set-Clipboard -Value ([Console]::In.ReadToEnd())"
    } else {
        "[Console]::OutputEncoding = [Text.UTF8Encoding]::new(); [Console]::Write((Get-Clipboard -Raw))"
    };
    vec![vec![
        "powershell.exe",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]]
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

/// Shown when `read_clipboard` fails (wl-paste also fails on an empty clipboard).
#[cfg(target_os = "linux")]
pub const CLIPBOARD_UNREADABLE: &str =
    "Could not read the clipboard: it is empty, or wl-clipboard, xclip or xsel is not installed";
#[cfg(windows)]
pub const CLIPBOARD_UNREADABLE: &str =
    "Could not read the clipboard: Windows PowerShell is unavailable or another app is using the clipboard";

/// The clipboard text; None if it could not be read.
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
#[cfg(target_os = "linux")]
pub fn open_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && spawn_detached("xdg-open", &[url])
}

#[cfg(windows)]
pub fn open_url(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && spawn_detached("rundll32.exe", &["url.dll,FileProtocolHandler", url])
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

#[cfg(target_os = "linux")]
pub fn open_path(path: &Path) -> bool {
    path.exists() && spawn_detached("xdg-open", &[&path.to_string_lossy()])
}

#[cfg(windows)]
pub fn open_path(path: &Path) -> bool {
    path.exists()
        && std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .is_ok()
}

#[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
pub fn open_window(url: &str, profile_dir: &Path) -> &'static str {
    // 1. Our own native window (built with the `webview` feature).
    #[cfg(all(target_os = "linux", feature = "webview"))]
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

#[cfg(windows)]
pub fn open_window(url: &str, profile_dir: &Path) -> &'static str {
    // 1. Our own window, when the WebView2 runtime is installed (Windows 11 has it).
    if wry::webview_version().is_ok() {
        if let Ok(exe) = std::env::current_exe() {
            use std::os::windows::process::CommandExt;
            if std::process::Command::new(exe)
                .args(["window", url])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .spawn()
                .is_ok()
            {
                return "webview";
            }
        }
    }
    // 2. An Edge app window, 3. the default browser.
    let edge = which("msedge.exe").or_else(|| {
        ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(|root| PathBuf::from(root).join("Microsoft/Edge/Application/msedge.exe"))
            .find(|path| path.is_file())
    });
    if let Some(edge) = edge {
        let _ = std::fs::create_dir_all(profile_dir);
        if std::process::Command::new(edge)
            .arg(format!("--app={url}"))
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .args([
                "--window-size=1120,780",
                "--no-first-run",
                "--no-default-browser-check",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return "msedge";
        }
    }
    if open_url(url) {
        return "default browser";
    }
    tracing::warn!("Open {url} in a browser to use AirBridge");
    "none"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_cannot_add_markup() {
        let doc = toast_xml(
            "“<b>x</b>\" would like to share",
            "a & b\u{202E}",
            None,
            &[("accept", "Accept"), ("default", "Show")],
            true,
        );
        assert_eq!(
            doc,
            "<toast launch=\"default\" scenario=\"reminder\"><visual><binding template=\"ToastGeneric\">\
             <text>“&lt;b&gt;x&lt;/b&gt;&quot; would like to share</text><text>a &amp; b</text>\
             </binding></visual><actions>\
             <action content=\"Accept\" arguments=\"accept\" activationType=\"foreground\"/>\
             </actions></toast>"
        );
    }

    #[cfg(target_os = "linux")]
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
