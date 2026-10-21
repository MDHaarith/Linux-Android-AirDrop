//! Native app window (WebKitGTK via wry) showing the local UI.
//!
//! Runs in its own process (`airbridge window <url>`), so closing the window
//! never stops the background service.

use anyhow::Result;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{DragDropEvent, WebViewBuilder};

enum UserEvent {
    Dropped(Vec<std::path::PathBuf>),
}

pub fn run(url: &str) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("AirBridge")
        .with_inner_size(LogicalSize::new(1120.0, 780.0))
        .with_min_inner_size(LogicalSize::new(720.0, 560.0))
        .build(&event_loop)?;

    let builder = WebViewBuilder::new()
        .with_url(url)
        .with_drag_drop_handler(move |event| match event {
            // Native drops carry real paths: no need to copy the files.
            DragDropEvent::Drop { paths, .. } => {
                let _ = proxy.send_event(UserEvent::Dropped(paths));
                true
            }
            _ => false,
        });

    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| anyhow::anyhow!("no GTK container"))?;
        builder.build_gtk(vbox)?
    };
    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::Dropped(paths)) => {
                let json = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into());
                let _ = webview.evaluate_script(&format!(
                    "window.airbridgeNativeDrop && window.airbridgeNativeDrop({json})"
                ));
            }
            _ => {}
        }
    });
}
