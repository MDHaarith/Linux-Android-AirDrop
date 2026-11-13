//! The drop zone on the screen edge for Windows: a thin always-on-top handle
//! (WebView2, page `ui/edge.html`) that opens into device bubbles while
//! something is dragged onto it.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Result};
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::WindowBuilderExtWindows;
use tao::window::WindowBuilder;
use wry::{DragDropEvent, WebViewBuilder};

/// Logical sizes of the closed handle and the open panel.
const HANDLE: (f64, f64) = (10.0, 140.0);
const PANEL: (f64, f64) = (300.0, 460.0);

enum UserEvent {
    Open,
    Close,
    Drag(DragDropEvent),
}

pub fn run(side: &str, url: &str) -> Result<()> {
    if wry::webview_version().is_err() {
        bail!("the WebView2 runtime is not installed");
    }
    let left = side == "left";
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let monitor = event_loop
        .primary_monitor()
        .ok_or_else(|| anyhow!("no screen"))?;
    let scale = monitor.scale_factor();
    let (origin, screen) = (monitor.position(), monitor.size());
    // Centered on the chosen edge of the main screen, in physical pixels.
    let place = move |(width, height): (f64, f64)| {
        let size = PhysicalSize::new((width * scale) as u32, (height * scale) as u32);
        let x = if left {
            origin.x
        } else {
            origin.x + screen.width as i32 - size.width as i32
        };
        let y = origin.y + (screen.height as i32 - size.height as i32) / 2;
        (PhysicalPosition::new(x, y), size)
    };
    let (position, size) = place(HANDLE);
    let window = WindowBuilder::new()
        .with_title("AirBridge drop zone")
        .with_decorations(false)
        .with_always_on_top(true)
        .with_skip_taskbar(true)
        .with_transparent(true)
        .with_resizable(false)
        .with_focused(false)
        .with_position(position)
        .with_inner_size(size)
        .build(&event_loop)?;

    let (ipc, drag) = (event_loop.create_proxy(), event_loop.create_proxy());
    // Files go through the native handler; text drags are left to the page.
    let files = AtomicBool::new(false);
    let data = crate::config::cache_dir().join("webview");
    let mut context = wry::WebContext::new(Some(data));
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .with_transparent(true)
        .with_initialization_script(format!(
            "window.airbridgeEdgeSide = {};",
            serde_json::to_string(side)?
        ))
        .with_ipc_handler(move |request| {
            let event = match request.body().as_str() {
                "open" => UserEvent::Open,
                "close" => UserEvent::Close,
                _ => return,
            };
            let _ = ipc.send_event(event);
        })
        .with_drag_drop_handler(move |event| {
            if let DragDropEvent::Enter { paths, .. } = &event {
                files.store(!paths.is_empty(), Ordering::Relaxed);
            }
            let handled = files.load(Ordering::Relaxed);
            if handled || matches!(event, DragDropEvent::Enter { .. }) {
                let _ = drag.send_event(UserEvent::Drag(event));
            }
            handled
        })
        .build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let resize = |(position, size): (PhysicalPosition<i32>, PhysicalSize<u32>)| {
            window.set_inner_size(size);
            window.set_outer_position(position);
        };
        // The page works in CSS pixels; drag positions come in physical ones.
        let css = |(x, y): (i32, i32)| (f64::from(x) / scale, f64::from(y) / scale);
        let script = match event {
            Event::UserEvent(UserEvent::Open) => return resize(place(PANEL)),
            Event::UserEvent(UserEvent::Close) => return resize(place(HANDLE)),
            Event::UserEvent(UserEvent::Drag(event)) => match event {
                DragDropEvent::Enter { position, .. } => {
                    resize(place(PANEL));
                    let (x, y) = css(position);
                    format!("window.airbridgeEdgeDrag?.({x}, {y})")
                }
                DragDropEvent::Over { position } => {
                    let (x, y) = css(position);
                    format!("window.airbridgeEdgeDrag?.({x}, {y})")
                }
                DragDropEvent::Drop { paths, position } => {
                    let (x, y) = css(position);
                    let json = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into());
                    format!("window.airbridgeEdgeDrop?.({json}, {x}, {y})")
                }
                DragDropEvent::Leave => "window.airbridgeEdgeDrag?.(null, null)".into(),
                _ => return,
            },
            _ => return,
        };
        let _ = webview.evaluate_script(&script);
    });
}
