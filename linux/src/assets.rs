//! UI files compiled into the binary.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

macro_rules! assets {
    ($($name:literal => $mime:literal),* $(,)?) => {
        pub fn get(path: &str) -> Option<(&'static [u8], &'static str)> {
            match path {
                $($name => Some((include_bytes!(concat!("../ui/", $name)), $mime)),)*
                _ => None,
            }
        }
    };
}

assets! {
    "index.html" => "text/html; charset=utf-8",
    "app.css" => "text/css; charset=utf-8",
    "app.js" => "text/javascript; charset=utf-8",
    "companion.html" => "text/html; charset=utf-8",
    "companion.css" => "text/css; charset=utf-8",
    "companion.js" => "text/javascript; charset=utf-8",
    "shared.css" => "text/css; charset=utf-8",
    "shared.js" => "text/javascript; charset=utf-8",
    "icon.svg" => "image/svg+xml",
}

pub fn response(path: &str) -> Response {
    match get(path) {
        Some((bytes, mime)) => (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, "no-cache"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
