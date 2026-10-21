//! What to do with something dropped on the screen edge, independent of
//! GTK: which formats to ask the drag source for, and how to read them.

use std::collections::HashMap;
use std::path::PathBuf;

/// Formats we understand, as drag targets.
pub const URI_LIST: &str = "text/uri-list";
pub const MOZ_URL: &str = "text/x-moz-url";
pub const NETSCAPE_URL: &str = "_NETSCAPE_URL";
/// Firefox's suggested file name for a dragged image.
pub const MOZ_FILE_NAME: &str = "application/x-moz-file-promise-dest-filename";
pub const TEXT_TARGETS: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];
pub const IMAGE_TARGETS: [&str; 7] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "image/svg+xml",
];

/// Everything the edge window registers as acceptable.
pub fn accepted_targets() -> Vec<&'static str> {
    let mut all = vec![URI_LIST, MOZ_URL, NETSCAPE_URL, MOZ_FILE_NAME];
    all.extend(IMAGE_TARGETS);
    all.extend(TEXT_TARGETS);
    all
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Local files and folders (from a file manager).
    Paths(Vec<PathBuf>),
    /// Image data handed over by the browser.
    Image {
        name: String,
        data: Vec<u8>,
    },
    /// A link: the service downloads it, or sends web pages as links.
    Url {
        url: String,
        name: Option<String>,
    },
    Text(String),
}

/// Decodes `text/x-moz-url` (UTF-16, "url\ntitle") or plain UTF-8.
pub fn decode_text(target: &str, data: &[u8]) -> String {
    let even = data.len().is_multiple_of(2);
    let utf16 = even
        && (target == MOZ_URL
            || (data.len() >= 2 && data.iter().skip(1).step_by(2).all(|&b| b == 0)));
    let text = if utf16 {
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(data).into_owned()
    };
    text.trim_start_matches('\u{feff}')
        .trim_end_matches('\0')
        .to_string()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The URIs in a `text/uri-list` (or the first line of a moz URL).
pub fn uris(target: &str, data: &[u8]) -> Vec<String> {
    let text = decode_text(target, data);
    if target == MOZ_URL || target == NETSCAPE_URL {
        return text
            .lines()
            .next()
            .map(|l| vec![l.trim().to_string()])
            .unwrap_or_default();
    }
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// `file:///home/me/a%20b.jpg` → `/home/me/a b.jpg` (local files only).
pub fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    path.starts_with('/')
        .then(|| PathBuf::from(percent_decode(path)))
}

fn url_extension(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    Some(ext.to_lowercase())
}

/// The image format to ask for: the one the page used, if offered.
fn best_image(offered: &[String], url: Option<&str>) -> Option<String> {
    let images: Vec<&String> = offered
        .iter()
        .filter(|t| IMAGE_TARGETS.contains(&t.as_str()))
        .collect();
    if images.is_empty() {
        return None;
    }
    let wanted = url.and_then(url_extension).map(|ext| match ext.as_str() {
        "jpg" | "jpeg" | "jfif" => "image/jpeg".to_string(),
        "svg" => "image/svg+xml".to_string(),
        other => format!("image/{other}"),
    });
    if let Some(wanted) = wanted {
        if let Some(t) = images.iter().find(|t| **t == &wanted) {
            return Some((*t).clone());
        }
    }
    images
        .iter()
        .find(|t| t.as_str() == "image/png")
        .or(images.first())
        .map(|t| (*t).clone())
}

/// What we already fetched from the drag source, by target.
pub type Received = HashMap<String, Vec<u8>>;

fn url_in(received: &Received) -> Option<String> {
    [URI_LIST, MOZ_URL, NETSCAPE_URL]
        .iter()
        .find_map(|t| received.get(*t).map(|d| uris(t, d)))
        .and_then(|list| list.into_iter().next())
}

/// The next format to request, or `None` when we know enough to decide.
pub fn next_target(offered: &[String], received: &Received) -> Option<String> {
    let has = |t: &str| offered.iter().any(|o| o == t);
    // 1. Links / file lists first: they tell us what the drop is.
    if let Some(t) = [URI_LIST, MOZ_URL, NETSCAPE_URL]
        .into_iter()
        .find(|t| has(t))
    {
        if !received.contains_key(t) {
            return Some(t.to_string());
        }
        let list = uris(t, &received[t]);
        if !list.is_empty() && list.iter().all(|u| file_path(u).is_some()) {
            return None; // files: nothing else needed
        }
    }
    let url = url_in(received);
    // 2. Image data from the browser (works even for pages behind a login).
    if let Some(image) = best_image(offered, url.as_deref()) {
        if !received.contains_key(&image) {
            return Some(image);
        }
        if has(MOZ_FILE_NAME) && !received.contains_key(MOZ_FILE_NAME) {
            return Some(MOZ_FILE_NAME.to_string());
        }
        return None;
    }
    // 3. Plain text, if there is no link.
    if url.is_none() {
        if let Some(t) = TEXT_TARGETS.iter().find(|t| has(t)) {
            if !received.contains_key(*t) {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn image_name(url: Option<&str>, suggested: Option<String>, mime: &str) -> String {
    let ext = match mime {
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        other => other.strip_prefix("image/").unwrap_or("png"),
    };
    let from_url = url
        .filter(|u| u.starts_with("http"))
        .and_then(|u| u.split(['?', '#']).next())
        .and_then(|p| p.rsplit('/').next())
        .map(percent_decode);
    let name = suggested
        .or(from_url)
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .unwrap_or_else(|| "image".into());
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name,
    };
    format!("{stem}.{ext}")
}

/// Turns what was received into something to send.
pub fn decide(received: &Received) -> Option<Payload> {
    if let Some(data) = received.get(URI_LIST) {
        let list = uris(URI_LIST, data);
        let paths: Vec<PathBuf> = list.iter().filter_map(|u| file_path(u)).collect();
        if !paths.is_empty() && paths.len() == list.len() {
            return Some(Payload::Paths(paths));
        }
    }
    let url = url_in(received).filter(|u| !u.starts_with("file:"));
    if let Some((mime, data)) = received
        .iter()
        .find(|(t, d)| IMAGE_TARGETS.contains(&t.as_str()) && !d.is_empty())
    {
        let suggested = received
            .get(MOZ_FILE_NAME)
            .map(|d| decode_text(MOZ_FILE_NAME, d))
            .filter(|n| !n.trim().is_empty());
        return Some(Payload::Image {
            name: image_name(url.as_deref(), suggested, mime),
            data: data.clone(),
        });
    }
    if let Some(url) = url {
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("data:") {
            return Some(Payload::Url { url, name: None });
        }
    }
    let text = TEXT_TARGETS
        .iter()
        .find_map(|t| received.get(*t).map(|d| decode_text(t, d)))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let single_link = (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && !trimmed.contains(char::is_whitespace);
    Some(if single_link {
        Payload::Url {
            url: trimmed.to_string(),
            name: None,
        }
    } else {
        Payload::Text(text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn files_from_a_file_manager() {
        let offer = offered(&[URI_LIST, "text/plain", "application/x-kde-cutselection"]);
        let mut got = Received::new();
        assert_eq!(next_target(&offer, &got).as_deref(), Some(URI_LIST));
        got.insert(
            URI_LIST.into(),
            b"file:///home/me/Holiday%20photos\r\nfile:///home/me/a.jpg\r\n".to_vec(),
        );
        assert_eq!(next_target(&offer, &got), None);
        assert_eq!(
            decide(&got),
            Some(Payload::Paths(vec![
                PathBuf::from("/home/me/Holiday photos"),
                PathBuf::from("/home/me/a.jpg")
            ]))
        );
    }

    #[test]
    fn image_from_firefox_prefers_the_original_format() {
        let offer = offered(&[
            MOZ_URL,
            "text/_moz_htmlinfo",
            "text/html",
            URI_LIST,
            "text/plain",
            MOZ_FILE_NAME,
            "image/png",
            "image/jpeg",
            "image/gif",
        ]);
        let mut got = Received::new();
        assert_eq!(next_target(&offer, &got).as_deref(), Some(URI_LIST));
        got.insert(
            URI_LIST.into(),
            b"https://cdn.example.com/p/cat%20nap.JPG?w=800".to_vec(),
        );
        assert_eq!(next_target(&offer, &got).as_deref(), Some("image/jpeg"));
        got.insert("image/jpeg".into(), vec![0xff, 0xd8, 0xff]);
        assert_eq!(next_target(&offer, &got).as_deref(), Some(MOZ_FILE_NAME));
        got.insert(MOZ_FILE_NAME.into(), b"cat nap.JPG".to_vec());
        assert_eq!(next_target(&offer, &got), None);
        assert_eq!(
            decide(&got),
            Some(Payload::Image {
                name: "cat nap.jpg".into(),
                data: vec![0xff, 0xd8, 0xff]
            })
        );
    }

    #[test]
    fn image_from_chrome_is_downloaded_by_url() {
        let offer = offered(&[
            URI_LIST,
            "text/html",
            "text/plain",
            "chromium/x-renderer-taint",
        ]);
        let mut got = Received::new();
        got.insert(
            URI_LIST.into(),
            b"https://images.example.org/hero.webp".to_vec(),
        );
        assert_eq!(next_target(&offer, &got), None);
        assert_eq!(
            decide(&got),
            Some(Payload::Url {
                url: "https://images.example.org/hero.webp".into(),
                name: None
            })
        );
    }

    #[test]
    fn moz_url_is_utf16() {
        let mut got = Received::new();
        got.insert(MOZ_URL.into(), utf16("https://example.com/page\nA page"));
        assert_eq!(
            decide(&got),
            Some(Payload::Url {
                url: "https://example.com/page".into(),
                name: None
            })
        );
    }

    #[test]
    fn selected_text() {
        let offer = offered(&["text/plain;charset=utf-8", "UTF8_STRING", "text/html"]);
        let mut got = Received::new();
        assert_eq!(
            next_target(&offer, &got).as_deref(),
            Some("text/plain;charset=utf-8")
        );
        got.insert(
            "text/plain;charset=utf-8".into(),
            "Meet at 7 ✨".as_bytes().to_vec(),
        );
        assert_eq!(next_target(&offer, &got), None);
        assert_eq!(decide(&got), Some(Payload::Text("Meet at 7 ✨".into())));

        let mut link = Received::new();
        link.insert("UTF8_STRING".into(), b"  https://example.com/x  ".to_vec());
        assert!(matches!(decide(&link), Some(Payload::Url { .. })));
        assert_eq!(decide(&Received::new()), None);
    }

    #[test]
    fn data_urls_and_names() {
        let mut got = Received::new();
        got.insert(
            URI_LIST.into(),
            b"data:image/png;base64,iVBORw0KGgo=".to_vec(),
        );
        assert!(matches!(decide(&got), Some(Payload::Url { .. })));
        assert_eq!(image_name(None, None, "image/png"), "image.png");
        assert_eq!(
            image_name(Some("https://x.io/a/"), None, "image/gif"),
            "image.gif"
        );
        assert_eq!(
            file_path("file://localhost/tmp/a"),
            Some(PathBuf::from("/tmp/a"))
        );
        assert_eq!(file_path("https://x"), None);
    }
}
