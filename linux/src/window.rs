//! Native app window (WebKitGTK or WebView2 via wry) showing the local UI.
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
    Drag(DragDropEvent),
    /// The page asked to close (the service is restarting after an update).
    Close,
    /// Something asked for AirBridge while its window is open.
    Focus,
}

pub fn run(url: &str) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let close = event_loop.create_proxy();
    #[cfg(windows)]
    let icon = {
        use tao::platform::windows::IconExtWindows;
        // The icon resource embedded by build.rs.
        tao::window::Icon::from_resource(1, None).ok()
    };
    #[cfg(not(windows))]
    let icon = None;
    // Fit small or scaled screens (1366×768 at 125% is only 1093×614 logical).
    let (width, height) = event_loop
        .primary_monitor()
        .map(|m| {
            let size = m.size().to_logical::<f64>(m.scale_factor());
            (size.width * 0.9, size.height * 0.85)
        })
        .unwrap_or((f64::MAX, f64::MAX));
    let window = WindowBuilder::new()
        .with_window_icon(icon)
        .with_title("AirBridge")
        .with_inner_size(LogicalSize::new(width.min(1120.0), height.min(780.0)))
        .with_min_inner_size(LogicalSize::new(width.min(720.0), height.min(560.0)))
        .build(&event_loop)?;

    // WebView2 would otherwise keep its data next to the exe, which may be read-only.
    let data = cfg!(windows).then(|| crate::config::cache_dir().join("webview"));
    let mut context = wry::WebContext::new(data);
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_ipc_handler(move |request| {
            let event = match request.body().as_str() {
                "close" => UserEvent::Close,
                "focus" => UserEvent::Focus,
                _ => return,
            };
            let _ = close.send_event(event);
        })
        .with_drag_drop_handler(move |event| {
            // WebKit also reports dragged browser URLs as paths. Let the DOM
            // handle those with the original URL instead of sending a file path.
            if let DragDropEvent::Drop { paths, .. } = &event {
                if paths.iter().any(|path| !path.is_absolute()) {
                    return false;
                }
            }
            let _ = proxy.send_event(UserEvent::Drag(event));
            true
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
            }
            | Event::UserEvent(UserEvent::Close) => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::Focus) => {
                window.set_minimized(false);
                window.set_focus();
            }
            Event::UserEvent(UserEvent::Drag(event)) => {
                // The page works in CSS pixels; WebView2 reports physical ones.
                let scale = if cfg!(windows) {
                    window.scale_factor()
                } else {
                    1.0
                };
                let css = |(x, y): (i32, i32)| (f64::from(x) / scale, f64::from(y) / scale);
                let script = match event {
                    DragDropEvent::Drop { paths, position } => {
                        let (x, y) = css(position);
                        let json = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into());
                        format!("window.airbridgeNativeDrop?.({json}, {x}, {y})")
                    }
                    DragDropEvent::Enter { position, .. } | DragDropEvent::Over { position } => {
                        let (x, y) = css(position);
                        format!("window.airbridgeNativeDrag?.({x}, {y})")
                    }
                    DragDropEvent::Leave => "window.airbridgeNativeDrag?.(null, null)".into(),
                    _ => return,
                };
                let _ = webview.evaluate_script(&script);
            }
            _ => {}
        }
    });
}
