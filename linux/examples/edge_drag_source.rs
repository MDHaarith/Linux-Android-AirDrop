//! A drag source for trying the screen-edge drop zone without a browser:
//!
//!     cargo run --features edge --example edge_drag_source -- image photo.png https://example.com/photo.png
//!     cargo run --features edge --example edge_drag_source -- files /path/a /path/b
//!     cargo run --features edge --example edge_drag_source -- url https://example.com/page
//!     cargo run --features edge --example edge_drag_source -- text "Hello"
//!
//! "image" offers what Firefox offers for an image: its URL and the pixels.

use gtk::prelude::*;
use gtk::{gdk, glib};

fn main() {
    gtk::init().expect("GTK");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("text");
    let rest = args.get(1..).unwrap_or_default().to_vec();

    let (targets, label): (Vec<&str>, String) = match mode {
        "files" => (
            vec!["text/uri-list", "text/plain"],
            format!("{} file(s)", rest.len()),
        ),
        "image" => (
            vec![
                "text/x-moz-url",
                "text/uri-list",
                "image/png",
                "image/jpeg",
                "text/plain",
            ],
            "an image".into(),
        ),
        "url" => (vec!["text/uri-list", "text/plain"], "a link".into()),
        _ => (
            vec!["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"],
            "some text".into(),
        ),
    };
    let entries: Vec<gtk::TargetEntry> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| gtk::TargetEntry::new(t, gtk::TargetFlags::OTHER_APP, i as u32))
        .collect();

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Drag source");
    window.set_default_size(260, 140);
    window.move_(40, 40);
    let button = gtk::Button::with_label(&format!("Drag {label} to the screen edge"));
    window.add(&button);
    button.drag_source_set(
        gdk::ModifierType::BUTTON1_MASK,
        &entries,
        gdk::DragAction::COPY,
    );

    let mode = mode.to_string();
    button.connect_drag_data_get(move |_, _, data, _, _| {
        let target = data.target();
        let name = target.name().to_string();
        let uri = |path: &str| {
            let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
            format!("file://{}", path.display().to_string().replace(' ', "%20"))
        };
        let bytes: Vec<u8> = match (mode.as_str(), name.as_str()) {
            ("files", "text/uri-list") => rest
                .iter()
                .map(|p| uri(p) + "\r\n")
                .collect::<String>()
                .into_bytes(),
            ("files", _) => rest.join("\n").into_bytes(),
            ("image", "text/x-moz-url") => format!("{}\nPhoto", rest[1])
                .encode_utf16()
                .flat_map(|u| u.to_le_bytes())
                .collect(),
            ("image", "image/png") | ("image", "image/jpeg") => {
                std::fs::read(&rest[0]).unwrap_or_default()
            }
            ("image", _) => rest[1].clone().into_bytes(),
            ("url", _) => rest[0].clone().into_bytes(),
            _ => rest.join(" ").into_bytes(),
        };
        data.set(&target, 8, &bytes);
    });
    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    window.show_all();
    gtk::main();
}
