//! The drop zone on the screen edge (`airbridge edge`): drag anything to the
//! side of the screen and drop it on a device. It runs as its own process,
//! started and restarted by the service, and talks to it over the local API.
//! Linux draws it with GTK; Windows shows `ui/edge.html` in WebView2.

#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "edge"), allow(dead_code))]
pub mod drop;

#[cfg(feature = "edge")]
mod api;
#[cfg(feature = "edge")]
mod panel;
#[cfg(windows)]
mod panel_windows;

#[cfg(feature = "edge")]
pub use panel::run;
#[cfg(windows)]
pub use panel_windows::run;

#[cfg(any(windows, feature = "edge"))]
pub use supervisor::supervise;

#[cfg(any(windows, feature = "edge"))]
mod supervisor {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::app::App;

    /// Exit code of `airbridge edge` when the desktop cannot host it.
    pub const UNSUPPORTED: i32 = 3;

    fn graphical_session() -> bool {
        cfg!(windows)
            || std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var_os("DISPLAY").is_some()
    }

    fn wanted(app: &App) -> Option<String> {
        let settings = app.settings();
        settings.edge_drop.then(|| settings.edge_side.clone())
    }

    /// Keeps the edge process running while the setting is on.
    pub fn supervise(app: Arc<App>) {
        if !graphical_session() {
            tracing::info!("No graphical session: screen-edge drop zone disabled");
            return;
        }
        tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                let Some(side) = wanted(&app) else {
                    tokio::select! {
                        _ = app.shutdown.cancelled() => return,
                        _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                    }
                };
                let Ok(exe) = std::env::current_exe() else {
                    return;
                };
                let mut command = tokio::process::Command::new(exe);
                command
                    .args(["edge", "--side", &side])
                    .stdin(std::process::Stdio::null())
                    .kill_on_drop(true);
                // The page signs in with a one-time code, like the app window.
                #[cfg(windows)]
                {
                    let Some(url) = app.page_url("edge") else {
                        return;
                    };
                    command.args(["--url", &url]);
                    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
                }
                let mut child = match command.spawn() {
                    Ok(child) => child,
                    Err(err) => {
                        tracing::warn!("Could not start the drop zone: {err}");
                        return;
                    }
                };
                let started = Instant::now();
                let exit = loop {
                    tokio::select! {
                        _ = app.shutdown.cancelled() => {
                            let _ = child.kill().await;
                            return;
                        }
                        status = child.wait() => break status.ok(),
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {
                            if wanted(&app).as_deref() != Some(side.as_str()) {
                                let _ = child.kill().await;
                                break None;
                            }
                        }
                    }
                };
                if let Some(status) = exit {
                    if status.code() == Some(UNSUPPORTED) {
                        tracing::info!("This desktop cannot show the screen-edge drop zone");
                        return;
                    }
                    failures = if started.elapsed() > Duration::from_secs(60) {
                        0
                    } else {
                        failures + 1
                    };
                    if failures >= 5 {
                        tracing::warn!("The drop zone keeps crashing; giving up");
                        return;
                    }
                    tracing::info!("Drop zone exited ({status}); restarting");
                    tokio::time::sleep(Duration::from_secs(u64::from(failures) * 2)).await;
                }
            }
        });
    }
}
