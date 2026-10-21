//! Downloads what was dragged out of a web page (an image URL, a `data:`
//! URL) so it can be sent as a file. Web pages themselves are sent as links.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty};
use hyper::{Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config;
use crate::sender::SendItem;

const MAX_REDIRECTS: usize = 5;
const MAX_SIZE: u64 = 8 * 1024 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

pub enum Fetched {
    /// Saved to the staging folder, ready to send (deleted afterwards).
    File(SendItem),
    /// An HTML page: better shared as a link than as a file.
    Page,
}

/// `name` is the file name the drag source suggested, if any.
pub async fn fetch(url: &str, name: Option<&str>) -> Result<Fetched> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (mime, data) = decode_data_url(rest)?;
        let name = file_name(name, None, &mime, "image");
        return Ok(Fetched::File(stage(&name, &data).await?));
    }
    let mut uri: Uri = url.parse().context("not a valid link")?;
    for _ in 0..=MAX_REDIRECTS {
        let response = get(&uri).await?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(hyper::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow!("redirect without a location"))?;
            uri = resolve(&uri, location)?;
            continue;
        }
        if status != StatusCode::OK {
            bail!("the server answered {status}");
        }
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v: &hyper::header::HeaderValue| v.to_str().ok())
                .map(str::to_string)
        };
        let mime = header(hyper::header::CONTENT_TYPE)
            .map(|m| m.split(';').next().unwrap_or("").trim().to_lowercase())
            .unwrap_or_default();
        if mime == "text/html" || mime == "application/xhtml+xml" {
            return Ok(Fetched::Page);
        }
        let disposition =
            header(hyper::header::CONTENT_DISPOSITION).and_then(|d| disposition_name(&d));
        let from_url = uri
            .path()
            .rsplit('/')
            .next()
            .map(percent_decode)
            .filter(|n| !n.is_empty());
        let suggested = name.map(str::to_string).or(disposition);
        let name = file_name(suggested.as_deref(), from_url.as_deref(), &mime, "download");
        let item = stage_stream(&name, response.into_body()).await?;
        return Ok(Fetched::File(item));
    }
    bail!("too many redirects")
}

fn decode_data_url(rest: &str) -> Result<(String, Vec<u8>)> {
    let (meta, payload) = rest
        .split_once(',')
        .ok_or_else(|| anyhow!("bad data URL"))?;
    let base64 = meta.ends_with(";base64");
    let mime = meta
        .trim_end_matches(";base64")
        .split(';')
        .next()
        .filter(|m| !m.is_empty())
        .unwrap_or("text/plain")
        .to_lowercase();
    let data = if base64 {
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .context("bad base64 in data URL")?
    } else {
        percent_decode(payload).into_bytes()
    };
    Ok((mime, data))
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

/// `filename*=UTF-8''…` or `filename="…"` from a Content-Disposition header.
fn disposition_name(header: &str) -> Option<String> {
    let mut plain = None;
    for part in header.split(';').map(str::trim) {
        if let Some(value) = part.strip_prefix("filename*=") {
            let value = value.splitn(3, '\'').nth(2).unwrap_or(value);
            return Some(percent_decode(value.trim_matches('"')));
        }
        if let Some(value) = part.strip_prefix("filename=") {
            plain = Some(value.trim_matches('"').to_string());
        }
    }
    plain.filter(|n| !n.is_empty())
}

/// A safe file name with an extension that matches the content.
pub fn file_name(
    suggested: Option<&str>,
    from_url: Option<&str>,
    mime: &str,
    fallback: &str,
) -> String {
    let base = suggested
        .or(from_url)
        .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n).trim().to_string())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| fallback.to_string());
    let base: String = base
        .chars()
        .map(|c| if c.is_control() || c == ':' { '_' } else { c })
        .take(180)
        .collect();
    let preferred = match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        "image/avif" => Some("avif"),
        "video/mp4" => Some("mp4"),
        "audio/mpeg" => Some("mp3"),
        "application/pdf" => Some("pdf"),
        "text/plain" => Some("txt"),
        _ => None,
    };
    let Some(wanted) = preferred.or_else(|| {
        mime_guess::get_mime_extensions_str(mime)
            .and_then(|exts| exts.first())
            .copied()
    }) else {
        return base;
    };
    let current = std::path::Path::new(&base)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    let fits = current.as_deref().is_some_and(|ext| {
        mime_guess::from_ext(ext)
            .iter()
            .any(|m| m.essence_str() == mime)
    });
    if fits {
        base
    } else {
        let stem = std::path::Path::new(&base)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(base.clone());
        format!("{stem}.{wanted}")
    }
}

fn staging_path(name: &str) -> PathBuf {
    let id = uuid::Uuid::new_v4().simple().to_string();
    config::cache_dir().join("staging").join(id).join(name)
}

async fn stage(name: &str, data: &[u8]) -> Result<SendItem> {
    let path = staging_path(name);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    tokio::fs::write(&path, data).await?;
    Ok(SendItem {
        path,
        name: name.to_string(),
        temporary: true,
    })
}

async fn stage_stream(name: &str, body: hyper::body::Incoming) -> Result<SendItem> {
    let path = staging_path(name);
    tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    let mut file = tokio::fs::File::create(&path).await?;
    let mut stream = body.into_data_stream();
    let mut size = 0u64;
    let result: Result<()> = async {
        while let Some(chunk) = timeout(IO_TIMEOUT, stream.next())
            .await
            .map_err(|_| anyhow!("the download stalled"))?
        {
            let chunk = chunk?;
            size += chunk.len() as u64;
            if size > MAX_SIZE {
                bail!("the file is too large");
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    if let Err(err) = result {
        let _ = tokio::fs::remove_dir_all(path.parent().unwrap()).await;
        return Err(err);
    }
    Ok(SendItem {
        path,
        name: name.to_string(),
        temporary: true,
    })
}

fn resolve(base: &Uri, location: &str) -> Result<Uri> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.parse()?);
    }
    let scheme = base.scheme_str().unwrap_or("https");
    let authority = base.authority().map(|a| a.as_str()).unwrap_or_default();
    let path = if location.starts_with('/') {
        location.to_string()
    } else {
        let dir = base.path().rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        format!("{dir}/{location}")
    };
    Ok(format!("{scheme}://{authority}{path}").parse()?)
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            let found = rustls_native_certs::load_native_certs();
            let (added, _) = roots.add_parsable_certificates(found.certs);
            if added == 0 {
                tracing::warn!("No system certificates found; HTTPS downloads will fail");
            }
            let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}

async fn get(uri: &Uri) -> Result<hyper::Response<hyper::body::Incoming>> {
    let https = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => bail!("only web links can be downloaded"),
    };
    let host = uri
        .host()
        .ok_or_else(|| anyhow!("link without a host"))?
        .to_string();
    let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
    let tcp = timeout(
        Duration::from_secs(10),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| anyhow!("{host} did not answer"))??;
    let request = Request::get(uri.path_and_query().map(|p| p.as_str()).unwrap_or("/"))
        .header(
            hyper::header::HOST,
            uri.authority().map(|a| a.as_str()).unwrap_or(&host),
        )
        .header(
            hyper::header::USER_AGENT,
            "Mozilla/5.0 (X11; Linux x86_64) AirBridge",
        )
        .header(hyper::header::ACCEPT, "*/*")
        // Image hosts often refuse requests that do not come from their site.
        .header(
            hyper::header::REFERER,
            format!("{}://{}/", if https { "https" } else { "http" }, host),
        )
        .body(Empty::<Bytes>::new())?;
    let response = if https {
        let name = rustls::pki_types::ServerName::try_from(host.clone())?;
        let tls = tokio_rustls::TlsConnector::from(tls_config())
            .connect(name, tcp)
            .await?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(tls)).await?;
        tokio::spawn(connection);
        timeout(IO_TIMEOUT, sender.send_request(request)).await
    } else {
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(tcp)).await?;
        tokio::spawn(connection);
        timeout(IO_TIMEOUT, sender.send_request(request)).await
    };
    Ok(response.map_err(|_| anyhow!("{host} did not answer"))??)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_get_matching_extensions() {
        assert_eq!(
            file_name(None, Some("cat.jpg"), "image/jpeg", "x"),
            "cat.jpg"
        );
        assert_eq!(
            file_name(None, Some("photo"), "image/png", "x"),
            "photo.png"
        );
        assert_eq!(
            file_name(None, Some("img.php"), "image/webp", "x"),
            "img.webp"
        );
        assert_eq!(file_name(None, None, "image/jpeg", "image"), "image.jpg");
        assert_eq!(file_name(Some("../../etc/passwd"), None, "", "x"), "passwd");
        assert_eq!(
            file_name(Some("Report.PDF"), None, "application/pdf", "x"),
            "Report.PDF"
        );
        assert_eq!(file_name(None, Some("data.bin"), "", "x"), "data.bin");
    }

    #[test]
    fn data_urls_decode() {
        let (mime, data) = decode_data_url("image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(&data[..4], b"\x89PNG");
        let (mime, data) = decode_data_url(",hello%20world").unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(data, b"hello world");
    }

    #[test]
    fn content_disposition_names() {
        assert_eq!(
            disposition_name("attachment; filename=\"a b.png\"").as_deref(),
            Some("a b.png")
        );
        assert_eq!(
            disposition_name("attachment; filename=x.png; filename*=UTF-8''caf%C3%A9.png")
                .as_deref(),
            Some("café.png")
        );
        assert_eq!(disposition_name("inline"), None);
    }

    #[test]
    fn redirects_resolve() {
        let base: Uri = "https://example.com/a/b.png".parse().unwrap();
        assert_eq!(
            resolve(&base, "/c.png").unwrap().to_string(),
            "https://example.com/c.png"
        );
        assert_eq!(
            resolve(&base, "d.png").unwrap().to_string(),
            "https://example.com/a/d.png"
        );
        assert_eq!(
            resolve(&base, "http://cdn.example.org/e.png")
                .unwrap()
                .to_string(),
            "http://cdn.example.org/e.png"
        );
    }

    #[tokio::test]
    async fn downloads_files_and_recognizes_pages() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    let mut buffer = vec![0u8; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                    let response: Vec<u8> = if request.starts_with("GET /old ") {
                        b"HTTP/1.1 302 Found\r\nLocation: /pics/cat\r\nContent-Length: 0\r\n\r\n"
                            .to_vec()
                    } else if request.starts_with("GET /pics/cat ") {
                        let mut r = b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\n\r\n".to_vec();
                        r.extend_from_slice(&[0xff, 0xd8, 0xff, 0xe0]);
                        r
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 6\r\n\r\n<html>".to_vec()
                    };
                    socket.write_all(&response).await.unwrap();
                });
            }
        });
        match fetch(&format!("http://127.0.0.1:{port}/old"), None)
            .await
            .unwrap()
        {
            Fetched::File(item) => {
                assert_eq!(item.name, "cat.jpg");
                assert!(item.temporary);
                assert_eq!(std::fs::read(&item.path).unwrap(), [0xff, 0xd8, 0xff, 0xe0]);
                let _ = std::fs::remove_dir_all(item.path.parent().unwrap());
            }
            Fetched::Page => panic!("expected a file"),
        }
        assert!(matches!(
            fetch(&format!("http://127.0.0.1:{port}/article"), None)
                .await
                .unwrap(),
            Fetched::Page
        ));
        match fetch("data:image/png;base64,iVBORw0KGgo=", None)
            .await
            .unwrap()
        {
            Fetched::File(item) => {
                assert_eq!(item.name, "image.png");
                let _ = std::fs::remove_dir_all(item.path.parent().unwrap());
            }
            Fetched::Page => panic!("expected a file"),
        }
        assert!(fetch("ftp://example.com/x", None).await.is_err());
    }
}
