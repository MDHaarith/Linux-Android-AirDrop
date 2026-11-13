//! Windows tray icon: open the window or the received files, quit. It shows that
//! AirBridge keeps receiving after its window is closed.

use std::sync::Arc;

use anyhow::Result;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::EventLoopBuilderExtWindows;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::app::App;
use crate::desktop;

enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Shutdown,
}

pub fn spawn(app: Arc<App>) {
    let runtime = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        if let Err(err) = run(app, runtime) {
            tracing::warn!("No tray icon: {err:#}");
        }
    });
}

fn run(app: Arc<App>, runtime: tokio::runtime::Handle) -> Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event()
        .with_any_thread(true)
        .build();
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));
    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));
    let proxy = event_loop.create_proxy();
    let shutdown = app.shutdown.clone();
    runtime.spawn(async move {
        shutdown.cancelled().await;
        let _ = proxy.send_event(UserEvent::Shutdown);
    });

    let open = MenuItem::new("Open AirBridge", true, None);
    let files = MenuItem::new("Open received files", true, None);
    let quit = MenuItem::new("Quit AirBridge", true, None);
    let alias = format!("Visible as “{}”", app.settings().alias);
    let menu = Menu::with_items(&[
        &open,
        &files,
        &PredefinedMenuItem::separator(),
        &MenuItem::new(alias, false, None),
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let mut tray = Some(
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("AirBridge keeps receiving in the background")
            // The icon resource embedded by build.rs.
            .with_icon(Icon::from_resource(1, None)?)
            .build()?,
    );

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Menu(event)) if event.id == open.id() => app.open_window(),
            Event::UserEvent(UserEvent::Menu(event)) if event.id == files.id() => {
                desktop::open_path(&app.settings().download_dir);
            }
            Event::UserEvent(UserEvent::Menu(event)) if event.id == quit.id() => {
                app.shutdown.cancel();
            }
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            })) => app.open_window(),
            // Remove the icon now: the process may end before this thread does.
            Event::UserEvent(UserEvent::Shutdown) => drop(tray.take()),
            _ => {}
        }
    })
}
