//! AirBridge: AirDrop-style sharing between Linux and Android.
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
mod events;
mod identity;
mod net;
mod peers;
mod protocol;
mod receiver;
mod sender;
mod storage;
mod transfers;
#[cfg(feature = "tray")]
mod tray;
mod ui;
#[cfg(feature = "webview")]
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
    about = "AirDrop-style file & text sharing between Linux and Android"
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
    Send {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Device name, IP or fingerprint.
        #[arg(long, short)]
        to: Option<String>,
        /// PIN, if the receiver requires one.
        #[arg(long)]
        pin: Option<String>,
    },
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
    #[cfg(feature = "webview")]
    #[command(hide = true)]
    Window { url: String },
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
    #[cfg(feature = "webview")]
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
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match runtime.block_on(run(cli)) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("airbridge: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command.unwrap_or(Command::Open) {
        Command::Open => serve(!cli.background).await,
        Command::Daemon => serve(false).await,
        Command::Send { paths, to, pin } => send(paths, to, pin).await,
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
        #[cfg(feature = "webview")]
        Command::Window { .. } => unreachable!("handled in main"),
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

async fn serve(open_window: bool) -> Result<ExitCode> {
    if running_instance().await.is_some() {
        if open_window {
            api(Method::POST, "/api/show", None).await?;
        } else {
            println!("AirBridge is already running.");
        }
        return Ok(ExitCode::SUCCESS);
    }

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

    #[cfg(feature = "tray")]
    tray::spawn(app.clone());

    let me = app.me();
    tracing::info!(
        "AirBridge {} is visible as “{}” on port {port} ({})",
        env!("CARGO_PKG_VERSION"),
        me.alias,
        if settings.https { "HTTPS" } else { "HTTP" }
    );
    tracing::info!("Fingerprint {}", me.fingerprint);
    if let Some(url) = app.ui_url() {
        tracing::info!("Window: {url}");
    }
    if let Some(url) = app.companion_url() {
        tracing::info!("Phone browser: {url}");
    }
    if open_window {
        app.open_window();
    }

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
        _ = app.shutdown.cancelled() => {}
    }
    app.shutdown.cancel();
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
    {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(exe)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .context("starting the AirBridge service")?;
    }
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

async fn send(paths: Vec<PathBuf>, to: Option<String>, pin: Option<String>) -> Result<ExitCode> {
    let paths = paths
        .iter()
        .map(|p| std::fs::canonicalize(p).with_context(|| format!("{}", p.display())))
        .collect::<Result<Vec<_>>>()?;
    ensure_running().await?;
    match to {
        None => {
            api(
                Method::POST,
                "/api/stage-paths",
                Some(json!({ "paths": paths })),
            )
            .await?;
            println!("Pick a device in the AirBridge window.");
            Ok(ExitCode::SUCCESS)
        }
        Some(to) => {
            let peer = wait_for_device(&to).await?;
            let body = json!({ "peer": peer, "paths": paths, "pin": pin });
            follow(api(Method::POST, "/api/send", Some(body)).await?).await
        }
    }
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
