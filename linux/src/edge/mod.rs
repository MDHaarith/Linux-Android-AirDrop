//! The drop zone on the screen edge (`airbridge edge`): drag anything to the
//! side of the screen and drop it on a device. It runs as its own process,
//! started and restarted by the service, and talks to it over the local API.

#[cfg_attr(not(feature = "edge"), allow(dead_code))]
pub mod drop;

#[cfg(feature = "edge")]
mod api;
#[cfg(feature = "edge")]
mod panel;

#[cfg(feature = "edge")]
pub use panel::run;

#[cfg(feature = "edge")]
pub use supervisor::supervise;

#[cfg(feature = "edge")]
mod supervisor {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::app::App;

    /// Exit code of `airbridge edge` when the desktop cannot host it.
    pub const UNSUPPORTED: i32 = 3;

    fn graphical_session() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
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
                let mut child = match tokio::process::Command::new(exe)
                    .args(["edge", "--side", &side])
                    .stdin(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                {
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
