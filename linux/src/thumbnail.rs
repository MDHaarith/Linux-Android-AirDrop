//! Small photo previews that travel with a request, so the other side can
//! show what is coming before accepting, like AirDrop does.
//!
//! LocalSend's `prepare-upload` has an optional `preview` per file; for
//! photos we put a `data:image/jpeg;base64,…` thumbnail there.

use std::io::{BufRead, Cursor, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageReader};

/// Photos bigger than this are not worth decoding just for a preview.
const MAX_SOURCE: u64 = 40 * 1024 * 1024;
/// Longest side of a preview, in pixels.
const SIZE: u32 = 360;
/// Previews we receive that are bigger than this are ignored.
pub const MAX_DATA_URL: usize = 300 * 1024;
/// How many files of a request get a preview.
pub const MAX_PREVIEWS: usize = 3;

pub fn is_previewable(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg" | "image/png" | "image/webp" | "image/gif"
    )
}

/// Decodes an image (only the formats compiled in: JPEG, PNG, WebP, GIF;
/// never SVG) within sane limits, upright, and re-encodes a small JPEG.
fn small_jpeg<R: BufRead + Seek>(reader: ImageReader<R>) -> Option<Vec<u8>> {
    let mut reader = reader.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    // Phone photos are often stored sideways with an EXIF rotation.
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    let thumbnail = image.thumbnail(SIZE, SIZE).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 72)
        .encode_image(&thumbnail)
        .ok()?;
    Some(jpeg)
}

/// A small upright JPEG of the photo at `path`.
pub fn jpeg(path: &Path) -> Option<Vec<u8>> {
    if std::fs::metadata(path).ok()?.len() > MAX_SOURCE {
        return None;
    }
    small_jpeg(ImageReader::open(path).ok()?)
}

/// A JPEG thumbnail of the photo at `path` as a data URL, upright.
pub fn data_url(path: &Path) -> Option<String> {
    Some(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(jpeg(path)?)
    ))
}

/// A preview another device sent, decoded and re-encoded by us, so only a
/// plain JPEG we made ever reaches other image parsers (the notification
/// server's).
pub fn reencode(preview: &str) -> Option<Vec<u8>> {
    small_jpeg(ImageReader::new(Cursor::new(decode(preview)?)))
}

/// Saves a notification picture privately in the cache; old ones are removed.
pub fn save_for_notification(name: &str, jpeg: &[u8]) -> Option<PathBuf> {
    let dir = crate::config::cache_dir().join("previews");
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(3600));
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let path = dir.join(format!("{name}.jpg"));
    crate::config::write_private(&path, jpeg).ok()?;
    Some(path)
}

/// A received preview we are willing to show: a small raster image.
pub fn accept(preview: &str) -> Option<&str> {
    let ok = preview.len() <= MAX_DATA_URL
        && [
            "data:image/jpeg;base64,",
            "data:image/png;base64,",
            "data:image/webp;base64,",
        ]
        .iter()
        .any(|prefix| preview.starts_with(prefix));
    ok.then_some(preview)
}

/// The image bytes inside an accepted preview (for notifications).
pub fn decode(preview: &str) -> Option<Vec<u8>> {
    let data = accept(preview)?.split_once(',')?.1;
    STANDARD.decode(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_small_upright_jpegs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        image::RgbImage::from_fn(1200, 600, |x, _| image::Rgb([(x % 256) as u8, 90, 200]))
            .save(&path)
            .unwrap();
        let url = data_url(&path).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        assert!(url.len() < MAX_DATA_URL);
        let bytes = decode(&url).unwrap();
        let preview = image::load_from_memory(&bytes).unwrap();
        assert_eq!((preview.width(), preview.height()), (360, 180));
        // A received preview is re-encoded before anything else parses it.
        let again = image::load_from_memory(&reencode(&url).unwrap()).unwrap();
        assert_eq!((again.width(), again.height()), (360, 180));
        assert!(reencode("data:image/png;base64,AAAA").is_none());
    }

    #[test]
    fn only_small_raster_previews_are_accepted() {
        assert!(accept("data:image/jpeg;base64,AAAA").is_some());
        assert!(accept("data:image/svg+xml;base64,AAAA").is_none());
        assert!(accept("Hello, this is a text message").is_none());
        let huge = format!("data:image/png;base64,{}", "A".repeat(MAX_DATA_URL));
        assert!(accept(&huge).is_none());
        assert!(data_url(Path::new("/nonexistent.jpg")).is_none());
    }
}
