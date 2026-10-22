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

/// Pages run only their own scripts (no inline or foreign ones, so an
/// injected name or message can never execute), load nothing from
/// elsewhere, and cannot be framed by other sites.
const PAGE_POLICY: &str =
    "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; connect-src 'self' ws: wss:; font-src 'self'; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'none'";

pub fn response(path: &str) -> Response {
    match get(path) {
        Some((bytes, mime)) => {
            let mut response = (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "no-cache"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (header::REFERRER_POLICY, "no-referrer"),
                ],
                bytes,
            )
                .into_response();
            if mime.starts_with("text/html") {
                let headers = response.headers_mut();
                headers.insert(
                    header::CONTENT_SECURITY_POLICY,
                    header::HeaderValue::from_static(PAGE_POLICY),
                );
                headers.insert(
                    header::X_FRAME_OPTIONS,
                    header::HeaderValue::from_static("DENY"),
                );
            }
            response
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
