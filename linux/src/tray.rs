//! System tray icon (StatusNotifierItem): open the window, send the clipboard, quit.

use std::sync::Arc;

use ksni::menu::{MenuItem, StandardItem, SubMenu};
use ksni::TrayMethods;

use crate::app::App;
use crate::desktop;
use crate::peers::PeerKind;

struct AirBridgeTray {
    app: Arc<App>,
    peers: Vec<(String, String)>,
    alias: String,
}

impl ksni::Tray for AirBridgeTray {
    fn id(&self) -> String {
        "airbridge".into()
    }

    fn title(&self) -> String {
        "AirBridge".into()
    }

    fn icon_name(&self) -> String {
        "airbridge".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon(32), icon(64)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "AirBridge".into(),
            description: format!(
                "Visible as “{}” · {} device(s) nearby",
                self.alias,
                self.peers.len()
            ),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.app.open_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let send_clipboard: Vec<MenuItem<Self>> = if self.peers.is_empty() {
            vec![StandardItem {
                label: "No devices nearby".into(),
                enabled: false,
                ..Default::default()
            }
            .into()]
        } else {
            self.peers
                .iter()
                .map(|(id, alias)| {
                    let id = id.clone();
                    StandardItem {
                        label: alias.clone(),
                        activate: Box::new(move |tray: &mut Self| {
                            let (app, id) = (tray.app.clone(), id.clone());
                            tokio::spawn(async move {
                                if let Some(text) =
                                    desktop::read_clipboard().await.filter(|t| !t.is_empty())
                                {
                                    let _ = app.send_text(&id, text, None);
                                }
                            });
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect()
        };
        vec![
            StandardItem {
                label: "Open AirBridge".into(),
                icon_name: "airbridge".into(),
                activate: Box::new(|tray: &mut Self| tray.app.open_window()),
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Send clipboard to".into(),
                submenu: send_clipboard,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open received files".into(),
                activate: Box::new(|tray: &mut Self| {
                    let dir = tray.app.settings().download_dir.clone();
                    desktop::open_path(&dir);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: format!("Visible as “{}”", self.alias),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|tray: &mut Self| tray.app.shutdown.cancel()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// A gradient disc with a white "beam" arc, drawn in code so the tray works
/// even when the icon is not installed in the theme.
fn icon(size: i32) -> ksni::Icon {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let r = (dx * dx + dy * dy).sqrt() / c;
            let t = (x + y) as f32 / (2 * size) as f32;
            let (mut red, mut green, mut blue) = (
                (99.0 + (236.0 - 99.0) * t) as u8,
                (102.0 + (72.0 - 102.0) * t) as u8,
                (241.0 + (153.0 - 241.0) * t) as u8,
            );
            // Two white arcs ("signal") above a dot.
            let (ax, ay) = (dx / c, (dy / c) + 0.25);
            let d = (ax * ax + ay * ay).sqrt();
            let upper = ay < -0.05;
            if (d < 0.14) || (upper && ((d - 0.42).abs() < 0.07 || (d - 0.68).abs() < 0.07)) {
                red = 255;
                green = 255;
                blue = 255;
            }
            let alpha = if r <= 0.96 {
                255
            } else if r <= 1.0 {
                ((1.0 - r) / 0.04 * 255.0) as u8
            } else {
                0
            };
            data.extend_from_slice(&[alpha, red, green, blue]);
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

fn peers(app: &App) -> Vec<(String, String)> {
    app.registry
        .list()
        .into_iter()
        .filter(|p| p.kind == PeerKind::LocalSend || p.kind == PeerKind::Web)
        .map(|p| (p.id().to_string(), p.info.alias.clone()))
        .collect()
}

pub fn spawn(app: Arc<App>) {
    tokio::spawn(async move {
        let tray = AirBridgeTray {
            app: app.clone(),
            peers: peers(&app),
            alias: app.me().alias,
        };
        let handle = match tray.spawn().await {
            Ok(handle) => handle,
            Err(err) => {
                tracing::info!("No system tray available ({err})");
                return;
            }
        };
        let mut events = app.bus.subscribe();
        loop {
            match events.recv().await {
                Ok(event) if matches!(event.name, "peer" | "peer-removed" | "settings") => {
                    let (list, alias) = (peers(&app), app.me().alias);
                    handle
                        .update(move |tray| {
                            tray.peers = list;
                            tray.alias = alias;
                        })
                        .await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}
