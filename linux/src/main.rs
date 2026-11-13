//! AirBridge: AirDrop-style sharing between Linux, Windows and Android.
//!
//! Speaks the LocalSend v2 protocol, so it works with the AirBridge Android
//! app and with any LocalSend client.

mod app;
mod assets;
mod client;
mod companion;
mod config;
mod desktop;
mod discovery;
// shortcut: no Windows edge drop zone; add a native panel when needed.
#[cfg(target_os = "linux")]
mod edge;
mod events;
mod fetch;
mod identity;
mod limits;
mod net;
mod peers;
mod protocol;
mod quickshare;
mod receiver;
mod sender;
mod storage;
mod thumbnail;
mod transfers;
#[cfg(all(target_os = "linux", feature = "tray"))]
// shortcut: no Windows tray icon; add a native tray when background controls are needed.
mod tray;
mod ui;
mod update;
#[cfg(all(target_os = "linux", feature = "webview"))]
mod window;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use clap::{Parser, Subcommand};
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rand::distributions::{Alphanumeric, DistString};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::app::{human_size, App, UiInfo};
use crate::discovery::Discovery;

#[derive(Parser)]
#[command(
    name = "airbridge",
    version,
    about = "AirDrop-style file & text sharing between Linux, Windows and Android"
)]
struct Cli {
    /// Start the service without opening the window (for autostart).
    #[arg(long, global = true)]
    background: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the AirBridge window (starts the service if needed). Default.
    Open,
    /// Run the service in the foreground without opening the window.
    Daemon,
    /// Send files or folders. Without --to the window opens so you can pick a device.
    Send(SendArgs),
    /// Send a text message ("-" reads standard input).
    Text {
        text: String,
        #[arg(long, short)]
        to: String,
        #[arg(long)]
        pin: Option<String>,
    },
    /// Send the clipboard contents to a device.
    Clipboard {
        #[arg(long, short)]
        to: String,
    },
    /// List nearby devices.
    Devices,
    /// Stop the running service.
    Quit,
    /// (internal) Native window for the given URL.
    #[cfg(all(target_os = "linux", feature = "webview"))]
    #[command(hide = true)]
    Window { url: String },
    /// (internal) The drop zone on the screen edge; the service starts it.
    #[cfg(all(target_os = "linux", feature = "edge"))]
    #[command(hide = true)]
    Edge {
        #[arg(long, default_value = "right")]
        side: String,
    },
}

#[derive(clap::Args)]
struct SendArgs {
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Device name, IP or fingerprint.
    #[arg(long, short)]
    to: Option<String>,
    /// PIN, if the receiver requires one.
    #[arg(long)]
    pin: Option<String>,
    /// Send each file's SHA-256 so the receiver checks the contents.
    #[arg(long)]
    verify: bool,
    /// With --verify: write "sha256  name" lines (sha256sum format) here afterwards.
    #[arg(long, requires = "verify", value_name = "FILE")]
    manifest: Option<PathBuf>,
    /// From folders, send only these: photos, videos, audio, documents and/or
    /// extensions such as raw,heic (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "KINDS")]
    only: Vec<String>,
    /// Skip folders with these names anywhere inside (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "NAMES")]
    exclude: Vec<String>,
    /// Receive everything inside this folder (a single folder takes this name).
    #[arg(long = "as", value_name = "FOLDER")]
    root: Option<String>,
    /// Only show how many files and bytes would be sent.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct Instance {
    port: u16,
    token: String,
    pid: u32,
}

fn instance_path() -> PathBuf {
    config::runtime_dir().join("instance.json")
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("AIRBRIDGE_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,zbus=warn,tracing=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    #[cfg(all(target_os = "linux", feature = "webview"))]
    if let Some(Command::Window { url }) = &cli.command {
        // GTK must own the main thread; no tokio runtime here.
        return match window::run(url) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("airbridge: {err:#}");
                ExitCode::FAILURE
            }
        };
    }
    init_logging();
    #[cfg(all(target_os = "linux", feature = "edge"))]
    if let Some(Command::Edge { side }) = &cli.command {
        // GTK must own the main thread, as for the window.
        return match edge::run(side) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("airbridge edge: {err:#}");
                ExitCode::from(3)
            }
        };
    }
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = match runtime.block_on(run(cli)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("airbridge: {err:#}");
            ExitCode::FAILURE
        }
    };
    if update::RESTART.load(std::sync::atomic::Ordering::Relaxed) {
        // Close every socket first so the new version can take the ports.
        runtime.shutdown_timeout(Duration::from_secs(3));
        if let Some(program) = update::program() {
            let mut command = std::process::Command::new(program);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            if let Err(err) = command.spawn() {
                eprintln!("airbridge: could not restart after the update: {err}");
            }
        }
    }
    code
}

async fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command.unwrap_or(Command::Open) {
        Command::Open => serve(!cli.background).await,
        Command::Daemon => serve(false).await,
        Command::Send(args) => send(args).await,
        Command::Text { text, to, pin } => {
            let text = if text == "-" {
                let mut buf = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
                buf
            } else {
                text
            };
            ensure_running().await?;
            let body = json!({ "peer": wait_for_device(&to).await?, "text": text, "pin": pin });
            follow(api(Method::POST, "/api/send-text", Some(body)).await?).await
        }
        Command::Clipboard { to } => {
            ensure_running().await?;
            let body = json!({ "peer": wait_for_device(&to).await? });
            follow(api(Method::POST, "/api/send-clipboard", Some(body)).await?).await
        }
        Command::Devices => {
            let fresh = ensure_running().await?;
            if fresh {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let (_, state) = api(Method::GET, "/api/state", None).await?;
            let peers = state["peers"].as_array().cloned().unwrap_or_default();
            if peers.is_empty() {
                println!("No devices found. Make sure the phone is on the same Wi-Fi and AirBridge/LocalSend is open.");
            }
            for p in peers {
                println!(
                    "{:<28} {:<16} {:<15} {}{}",
                    p["alias"].as_str().unwrap_or("?"),
                    p["deviceModel"].as_str().unwrap_or(""),
                    p["ip"].as_str().unwrap_or(""),
                    p["kind"].as_str().unwrap_or(""),
                    if p["trusted"].as_bool() == Some(true) {
                        " ★"
                    } else {
                        ""
                    },
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Quit => {
            if running_instance().await.is_some() {
                api(Method::POST, "/api/quit", None).await?;
                println!("AirBridge stopped.");
            } else {
                println!("AirBridge is not running.");
            }
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(all(target_os = "linux", feature = "webview"))]
        Command::Window { .. } => unreachable!("handled in main"),
        #[cfg(all(target_os = "linux", feature = "edge"))]
        Command::Edge { .. } => unreachable!("handled in main"),
    }
}

// ------------------------------------------------------------------ service

async fn bind(port: u16, attempts: u16, host: &str) -> Result<(TcpListener, u16)> {
    let mut last_error = None;
    for offset in 0..attempts {
        let candidate = port.saturating_add(offset);
        match TcpListener::bind((host, candidate)).await {
            Ok(listener) => {
                let bound = listener_port(&listener)?;
                return Ok((listener, bound));
            }
            Err(err) => last_error = Some(err),
        }
    }
    Err(anyhow!(
        "cannot listen on {host}:{port}: {}",
        last_error.map(|e| e.to_string()).unwrap_or_default()
    ))
}

fn listener_port(listener: &TcpListener) -> Result<u16> {
    Ok(listener.local_addr()?.port())
}

pub async fn start_quick_share(app: &std::sync::Arc<App>) {
    if app.quick_share.lock().unwrap().is_some() {
        return;
    }
    match quickshare::Service::start(app).await {
        Ok(service) => *app.quick_share.lock().unwrap() = Some(service),
        Err(err) => tracing::warn!("Quick Share unavailable: {err:#}"),
    }
}

pub fn stop_quick_share(app: &App) {
    if let Some(service) = app.quick_share.lock().unwrap().take() {
        service.stop();
    }
}

/// Held by the service for its whole life, so only one service owns a profile
/// (its identity, settings and instance file). The OS releases it when the
/// process ends, even after a crash. None: another service holds it.
fn lock_profile() -> Result<Option<std::fs::File>> {
    let dir = config::config_dir();
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("service.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(err)) => Err(err.into()),
    }
}

async fn already_running(open_window: bool) -> Result<ExitCode> {
    if open_window {
        api(Method::POST, "/api/show", None).await?;
    } else {
        println!("AirBridge is already running.");
    }
    Ok(ExitCode::SUCCESS)
}

async fn serve(open_window: bool) -> Result<ExitCode> {
    if running_instance().await.is_some() {
        return already_running(open_window).await;
    }
    let Some(_profile_lock) = lock_profile()? else {
        // Launched at the same moment as another service: let that one finish starting.
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if running_instance().await.is_some() {
                return already_running(open_window).await;
            }
        }
        bail!("another AirBridge service is starting with this profile");
    };

    #[cfg(windows)]
    update::remove_old_copy();
    let app = App::new()?;
    let _ = std::fs::remove_dir_all(config::cache_dir().join("staging"));
    let settings = app.settings().clone();

    // LocalSend API (HTTPS) — what other devices talk to.
    let (listener, port) = bind(settings.port, 10, "0.0.0.0").await?;
    if port != settings.port {
        tracing::warn!(
            "Port {} is busy (is LocalSend running?); using {port}",
            settings.port
        );
    }
    app.port.store(port, Ordering::Relaxed);
    let tls = settings.https.then(|| app.identity.server_config());
    tokio::spawn(receiver::serve(app.clone(), listener, tls));

    // Discovery (multicast + register).
    let discovery = Discovery::start(app.clone()).await;
    let _ = app.discovery.set(discovery);

    // Google Quick Share, so stock Android phones see this computer.
    if settings.quick_share {
        start_quick_share(&app).await;
    }

    // Desktop UI, reachable from this computer only.
    let (ui_listener, ui_port) = match bind(settings.ui_port, 1, "127.0.0.1").await {
        Ok(bound) => bound,
        Err(_) => bind(0, 1, "127.0.0.1").await?,
    };
    let token = Alphanumeric.sample_string(&mut rand::thread_rng(), 32);
    let _ = app.ui.set(UiInfo {
        port: ui_port,
        token: token.clone(),
    });
    let instance = Instance {
        port: ui_port,
        token,
        pid: std::process::id(),
    };
    config::write_private(&instance_path(), &serde_json::to_vec(&instance)?)?;
    let ui_router = ui::router(app.clone());
    tokio::spawn(async move {
        if let Err(err) = axum::serve(ui_listener, ui_router).await {
            tracing::error!("UI server stopped: {err}");
        }
    });

    // Browser companion for phones without the app.
    match bind(settings.web_port, 10, "0.0.0.0").await {
        Ok((listener, web_port)) => {
            app.web_port.store(web_port, Ordering::Relaxed);
            tokio::spawn(companion::serve(app.clone(), listener));
        }
        Err(err) => tracing::warn!("Web companion unavailable: {err}"),
    }

    #[cfg(all(target_os = "linux", feature = "tray"))]
    tray::spawn(app.clone());

    #[cfg(all(target_os = "linux", feature = "edge"))]
    edge::supervise(app.clone());

    let me = app.me();
    tracing::info!(
        "AirBridge {} is visible as “{}” on port {port} ({})",
        env!("CARGO_PKG_VERSION"),
        me.alias,
        if settings.https { "HTTPS" } else { "HTTP" }
    );
    tracing::info!("Fingerprint {}", me.fingerprint);
    // Addresses only, never the secrets that open them.
    if let Some(ui) = app.ui.get() {
        tracing::info!("Window on http://127.0.0.1:{}/", ui.port);
    }
    if let Some(url) = app.companion_url() {
        let address = url.split('#').next().unwrap_or_default();
        tracing::info!("Phone browser page on {address} (pair with the QR code)");
    }
    if open_window {
        app.open_window();
    }

    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let terminated = async {
        #[cfg(unix)]
        terminate.recv().await;
        #[cfg(not(unix))]
        std::future::pending::<()>().await;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminated => {}
        _ = app.shutdown.cancelled() => {}
    }
    app.shutdown.cancel();
    stop_quick_share(&app);
    let _ = std::fs::remove_file(instance_path());
    tracing::info!("Bye");
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------- CLI

fn read_instance() -> Option<Instance> {
    serde_json::from_slice(&std::fs::read(instance_path()).ok()?).ok()
}

async fn running_instance() -> Option<Instance> {
    let instance = read_instance()?;
    match request(&instance, Method::GET, "/api/ping", None).await {
        Ok((200, _)) => Some(instance),
        _ => None,
    }
}

/// Starts the service in the background if needed. Returns true if it was started now.
async fn ensure_running() -> Result<bool> {
    if running_instance().await.is_some() {
        return Ok(false);
    }
    let exe = std::env::current_exe().context("locating airbridge")?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // shortcut: no Windows autostart; launch manually until a per-user installer exists.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command.spawn().context("starting the AirBridge service")?;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if running_instance().await.is_some() {
            return Ok(true);
        }
    }
    bail!("the AirBridge service did not start (run `airbridge daemon` to see why)")
}

async fn request(
    instance: &Instance,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(u16, Value)> {
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", instance.port)).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(connection);
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{}", instance.port))
        .header("x-airbridge-token", &instance.token)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(payload)))?;
    let response = sender.send_request(request).await?;
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn api(method: Method, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
    let instance = read_instance().ok_or_else(|| anyhow!("AirBridge is not running"))?;
    request(&instance, method, path, body).await
}

/// Waits (briefly) for a device matching `query` to be discovered; returns its id.
async fn wait_for_device(query: &str) -> Result<String> {
    let q = query.to_lowercase();
    for attempt in 0..40 {
        let (_, state) = api(Method::GET, "/api/state", None).await?;
        let peers = state["peers"].as_array().cloned().unwrap_or_default();
        let exact = peers.iter().find(|p| {
            ["id", "alias", "ip"]
                .iter()
                .any(|k| p[*k].as_str().is_some_and(|v| v.to_lowercase() == q))
        });
        let partial: Vec<&Value> = peers
            .iter()
            .filter(|p| {
                p["alias"]
                    .as_str()
                    .is_some_and(|a| a.to_lowercase().contains(&q))
            })
            .collect();
        if let Some(p) = exact.or((partial.len() == 1).then(|| partial[0])) {
            return Ok(p["id"].as_str().unwrap_or_default().to_string());
        }
        if partial.len() > 1 {
            bail!("“{query}” matches several devices; be more specific");
        }
        if attempt == 0 {
            eprintln!("Looking for “{query}”…");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("no device called “{query}” found (try `airbridge devices`)")
}

async fn send(args: SendArgs) -> Result<ExitCode> {
    let paths = args
        .paths
        .iter()
        .map(|p| std::fs::canonicalize(p).with_context(|| format!("{}", p.display())))
        .collect::<Result<Vec<_>>>()?;
    let picky =
        args.verify || !args.only.is_empty() || !args.exclude.is_empty() || args.root.is_some();
    const KINDS: [&str; 4] = ["photos", "videos", "audio", "documents"];
    let (kinds, extensions): (Vec<String>, Vec<String>) = args
        .only
        .iter()
        .map(|o| o.trim().to_lowercase())
        .filter(|o| !o.is_empty())
        .partition(|o| KINDS.contains(&o.as_str()));
    let options = json!({
        "paths": paths,
        "pin": args.pin,
        "verify": args.verify,
        "kinds": kinds,
        "extensions": extensions,
        "exclude": args.exclude,
        "root": args.root,
    });
    ensure_running().await?;
    if args.dry_run {
        let mut body = options;
        body["dryRun"] = true.into();
        let (status, response) = api(Method::POST, "/api/send", Some(body)).await?;
        if status != 200 {
            bail!(
                "{}",
                response["message"].as_str().unwrap_or("request failed")
            );
        }
        println!(
            "{} file(s), {}",
            response["count"].as_u64().unwrap_or(0),
            human_size(response["bytes"].as_u64().unwrap_or(0))
        );
        return Ok(ExitCode::SUCCESS);
    }
    let Some(to) = args.to else {
        if picky {
            bail!("--verify, --only, --exclude and --as need --to");
        }
        api(
            Method::POST,
            "/api/stage-paths",
            Some(json!({ "paths": paths })),
        )
        .await?;
        println!("Pick a device in the AirBridge window.");
        return Ok(ExitCode::SUCCESS);
    };
    let mut body = options;
    body["peer"] = wait_for_device(&to).await?.into();
    let response = api(Method::POST, "/api/send", Some(body)).await?;
    let id = response.1["id"].as_str().unwrap_or_default().to_string();
    let code = follow(response).await?;
    if let Some(manifest) = args.manifest {
        let (_, t) = api(Method::GET, &format!("/api/transfers/{id}"), None).await?;
        let lines: String = t["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                Some(format!(
                    "{}  {}\n",
                    f["sha256"].as_str()?,
                    f["name"].as_str()?
                ))
            })
            .collect();
        std::fs::write(&manifest, lines).with_context(|| format!("{}", manifest.display()))?;
    }
    Ok(code)
}

/// Prints the progress of a transfer started through the API until it ends.
async fn follow((status, response): (u16, Value)) -> Result<ExitCode> {
    if status != 200 {
        bail!(
            "{}",
            response["message"].as_str().unwrap_or("request failed")
        );
    }
    let id = response["id"].as_str().unwrap_or_default().to_string();
    let mut announced_wait = false;
    loop {
        let (_, t) = api(Method::GET, &format!("/api/transfers/{id}"), None).await?;
        let status = t["status"].as_str().unwrap_or("failed");
        let (done, total) = (
            t["done"].as_u64().unwrap_or(0),
            t["total"].as_u64().unwrap_or(0),
        );
        match status {
            "waiting" if !announced_wait => {
                eprintln!(
                    "Waiting for {} to accept…",
                    t["peer"]["alias"].as_str().unwrap_or("the device")
                );
                announced_wait = true;
            }
            "transferring" => {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                let bar: String = (0..30)
                    .map(|i| if i * 100 / 30 < pct { '█' } else { '░' })
                    .collect();
                eprint!(
                    "\r{bar} {pct:>3}%  {} / {}  {}/s   ",
                    human_size(done),
                    human_size(total),
                    human_size(t["speed"].as_f64().unwrap_or(0.0) as u64)
                );
                let _ = std::io::stderr().flush();
            }
            "done" => {
                eprintln!(
                    "\r✓ Sent to {}{}",
                    t["peer"]["alias"].as_str().unwrap_or("device"),
                    " ".repeat(50)
                );
                return Ok(ExitCode::SUCCESS);
            }
            "declined" | "cancelled" | "failed" => {
                eprintln!();
                bail!("{}", t["error"].as_str().unwrap_or(status));
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
