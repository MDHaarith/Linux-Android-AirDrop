//! Sending to an Android phone's built-in Quick Share: we connect to it.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use prost::Message;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::crypto::{alert_message, ClientHandshake};
use super::inbound::pair;
use super::link::{Event, Link};
use super::proto::location::nearby::connections::{
    connection_response_frame::ResponseStatus, payload_transfer_frame::payload_header::PayloadType,
    v1_frame::FrameType,
};
use super::proto::sharing::nearby::{
    self as sharing, connection_response_frame::Status as Response, file_metadata, text_metadata,
    v1_frame::FrameType as SharingType,
};
use super::wire::{self, DeviceKind, EndpointInfo, Incoming};
use crate::app::App;
use crate::peers::Peer;
use crate::protocol::FileDto;
use crate::sender::SendItem;
use crate::transfers::{now, Direction, FileState, FileStatus, Status, Transfer};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the person holding the phone has to accept.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

enum Content {
    Files(Vec<SendItem>),
    Text(String),
}

pub fn send_files(app: &Arc<App>, peer: Peer, items: Vec<SendItem>) -> Result<String, String> {
    if items.is_empty() {
        return Err("Nothing to send".into());
    }
    let mut files = Vec::with_capacity(items.len());
    for item in &items {
        let dto = FileDto::for_path(&item.path, &item.name)
            .map_err(|e| format!("{}: {e}", item.path.display()))?;
        files.push(FileState::new(
            dto,
            Some(item.path.to_string_lossy().into_owned()),
        ));
    }
    let mut transfer = Transfer::new(
        Direction::Out,
        peer.id(),
        app.registry.peer_json(&peer),
        files,
    );
    transfer.cleanup = items
        .iter()
        .filter(|i| i.temporary)
        .map(|i| i.path.clone())
        .collect();
    Ok(start(app, transfer, peer, Content::Files(items)))
}

pub fn send_text(app: &Arc<App>, peer: Peer, text: String) -> String {
    let dto = FileDto::for_text(&text);
    let mut transfer = Transfer::new(
        Direction::Out,
        peer.id(),
        app.registry.peer_json(&peer),
        vec![FileState::new(dto, None)],
    );
    transfer.message = Some(text.clone());
    start(app, transfer, peer, Content::Text(text))
}

fn start(app: &Arc<App>, mut transfer: Transfer, peer: Peer, content: Content) -> String {
    transfer.peer_ip = Some(peer.ip);
    let id = app.transfers.lock().unwrap().add(transfer);
    let cancel = CancellationToken::new();
    app.register_cancel(&id, cancel.clone());
    let addr = SocketAddr::new(peer.ip, peer.info.port);
    let (app, transfer_id) = (app.clone(), id.clone());
    tokio::spawn(async move {
        let result = run(&app, &transfer_id, addr, content, &cancel).await;
        app.unregister_cancel(&transfer_id);
        let (status, error) = match result {
            Ok(status) => (status, None),
            Err(_) if cancel.is_cancelled() => (Status::Cancelled, None),
            Err(err) => {
                tracing::warn!("Quick Share send to {addr} failed: {err:#}");
                (Status::Failed, Some(format!("{err}")))
            }
        };
        if status == Status::Done {
            let mut transfers = app.transfers.lock().unwrap();
            if let Some(t) = transfers.get_mut(&transfer_id) {
                for f in t.files.iter_mut() {
                    f.status = FileStatus::Done;
                    f.done = f.size;
                }
            }
        }
        app.finish(&transfer_id, status, error);
    });
    id
}

fn set_pin(app: &App, id: &str, pin: &str) {
    let mut transfers = app.transfers.lock().unwrap();
    if let Some(t) = transfers.get_mut(id) {
        t.peer["pin"] = pin.into();
    }
    transfers.emit(id, true);
}

async fn connect(app: &App, addr: SocketAddr) -> Result<(Link, String)> {
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("The phone did not answer"))?
        .context("Could not reach the phone")?;
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let handshake = async {
        let me = EndpointInfo {
            kind: DeviceKind::Laptop,
            name: Some(app.settings().alias.clone()),
        };
        let endpoint_id = app
            .quick_share
            .lock()
            .unwrap()
            .as_ref()
            .map(|qs| qs.endpoint_id)
            .unwrap_or_else(wire::random_endpoint_id);
        let request = wire::connection_request(&endpoint_id, &me);
        wire::write_frame(&mut writer, &request.encode_to_vec()).await?;
        let client = ClientHandshake::new();
        wire::write_frame(&mut writer, client.client_init()).await?;
        let server_init = wire::read_frame(&mut reader).await?;
        let (session, client_finish) = match client.finish(&server_init) {
            Ok(step) => step,
            Err(err) => {
                let _ = wire::write_frame(&mut writer, &alert_message(&err)).await;
                return Err(err.into());
            }
        };
        wire::write_frame(&mut writer, &client_finish).await?;
        wire::write_frame(&mut writer, &wire::connection_response().encode_to_vec()).await?;
        let (kind, v1) = wire::decode_offline(&wire::read_frame(&mut reader).await?)?;
        let accepted =
            v1.connection_response.and_then(|r| r.response) == Some(ResponseStatus::Accept as i32);
        if kind != FrameType::ConnectionResponse || !accepted {
            bail!("The phone refused the connection");
        }
        anyhow::Ok(session)
    };
    let session = timeout(HANDSHAKE_TIMEOUT, handshake)
        .await
        .map_err(|_| anyhow!("The phone did not finish connecting"))??;
    let pin = session.pin.clone();
    Ok((Link::start(reader, writer, session), pin))
}

fn file_metadata(item: &SendItem, size: u64, mime: &str, payload_id: i64) -> sharing::FileMetadata {
    let kind = if mime.starts_with("image/") {
        file_metadata::Type::Image
    } else if mime.starts_with("video/") {
        file_metadata::Type::Video
    } else if mime.starts_with("audio/") {
        file_metadata::Type::Audio
    } else if mime == "application/vnd.android.package-archive" {
        file_metadata::Type::AndroidApp
    } else {
        file_metadata::Type::Unknown
    };
    let (folder, name) = match item.name.rsplit_once('/') {
        Some((folder, name)) => (Some(folder.to_string()), name.to_string()),
        None => (None, item.name.clone()),
    };
    sharing::FileMetadata {
        name: Some(name),
        r#type: Some(kind as i32),
        payload_id: Some(payload_id),
        size: Some(size as i64),
        mime_type: Some(mime.to_string()),
        id: Some(wire::random_payload_id()),
        parent_folder: folder,
        ..Default::default()
    }
}

fn text_metadata(text: &str, payload_id: i64) -> sharing::TextMetadata {
    let trimmed = text.trim();
    let is_url = (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && !trimmed.contains(char::is_whitespace);
    let title: String = if is_url {
        trimmed
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or(trimmed)
            .to_string()
    } else {
        trimmed.chars().take(40).collect()
    };
    sharing::TextMetadata {
        text_title: Some(title),
        r#type: Some(if is_url {
            text_metadata::Type::Url
        } else {
            text_metadata::Type::Text
        } as i32),
        payload_id: Some(payload_id),
        size: Some(text.len() as i64),
        id: Some(wire::random_payload_id()),
        ..Default::default()
    }
}

/// Fails if the phone hung up or cancelled; otherwise does not wait.
fn check_peer(link: &mut Link) -> Result<()> {
    while let Some(event) = link.try_next() {
        match event {
            Event::Closed(reason) => {
                bail!("{}", reason.unwrap_or_else(|| "The phone hung up".into()))
            }
            Event::Payload(Incoming::Bytes { data, .. }) => {
                let frame = sharing::Frame::decode(data.as_slice()).ok();
                if frame.is_some_and(|f| wire::sharing_type(&f) == SharingType::Cancel) {
                    bail!("Cancelled on the phone");
                }
            }
            Event::Payload(Incoming::FileChunk { .. }) => {}
        }
    }
    Ok(())
}

async fn run(
    app: &Arc<App>,
    id: &str,
    addr: SocketAddr,
    content: Content,
    cancel: &CancellationToken,
) -> Result<Status> {
    let (mut link, pin) = tokio::select! {
        _ = cancel.cancelled() => return Ok(Status::Cancelled),
        connected = connect(app, addr) => connected?,
    };
    set_pin(app, id, &pin);
    tokio::select! {
        _ = cancel.cancelled() => return Ok(Status::Cancelled),
        paired = pair(&mut link) => paired?,
    }

    // Describe what is coming; the phone shows it and asks.
    let mut intro = sharing::IntroductionFrame::default();
    let mut files = Vec::new();
    match &content {
        Content::Files(items) => {
            let states: Vec<(u64, String)> = {
                let transfers = app.transfers.lock().unwrap();
                let t = transfers.get(id).context("transfer vanished")?;
                t.files
                    .iter()
                    .map(|f| (f.size, f.file_type.clone()))
                    .collect()
            };
            for (index, (item, (size, mime))) in items.iter().zip(states).enumerate() {
                let payload_id = wire::random_payload_id();
                intro
                    .file_metadata
                    .push(file_metadata(item, size, &mime, payload_id));
                files.push((index, item.path.clone(), size, payload_id));
            }
        }
        Content::Text(text) => {
            intro
                .text_metadata
                .push(text_metadata(text, wire::random_payload_id()));
        }
    }
    let text_payload = intro.text_metadata.first().and_then(|m| m.payload_id);
    link.send_sharing(&wire::introduction(intro)).await?;

    let answer = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = link.send_sharing(&wire::cancel()).await;
            link.close().await;
            return Ok(Status::Cancelled);
        }
        answer = timeout(ANSWER_TIMEOUT, link.expect(SharingType::Response)) => {
            answer.map_err(|_| anyhow!("Nobody answered on the phone"))??
        }
    };
    let status = answer
        .v1
        .and_then(|v1| v1.connection_response)
        .and_then(|r| r.status)
        .and_then(|s| Response::try_from(s).ok())
        .unwrap_or(Response::Unknown);
    match status {
        Response::Accept => {}
        Response::Reject => {
            link.close().await;
            return Ok(Status::Declined);
        }
        Response::NotEnoughSpace => bail!("Not enough space on the phone"),
        Response::UnsupportedAttachmentType => bail!("The phone cannot receive this"),
        Response::TimedOut => bail!("Nobody answered on the phone"),
        Response::Unknown => bail!("Unexpected answer from the phone"),
    }

    {
        let mut transfers = app.transfers.lock().unwrap();
        if let Some(t) = transfers.get_mut(id) {
            t.status = Status::Transferring;
            t.started = Some(now());
        }
        transfers.emit(id, true);
    }

    if let (Content::Text(text), Some(payload_id)) = (&content, text_payload) {
        for frame in wire::bytes_payload(payload_id, text.as_bytes().to_vec()) {
            link.send(&frame).await?;
        }
    }
    for (index, path, size, payload_id) in files {
        send_file(app, id, &mut link, cancel, index, &path, size, payload_id).await?;
    }
    link.close().await;
    Ok(Status::Done)
}

#[allow(clippy::too_many_arguments)]
async fn send_file(
    app: &Arc<App>,
    id: &str,
    link: &mut Link,
    cancel: &CancellationToken,
    index: usize,
    path: &std::path::Path,
    size: u64,
    payload_id: i64,
) -> Result<()> {
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("Could not open {}", path.display()))?;
    let total = size as i64;
    let mut offset = 0i64;
    let mut buffer = vec![0u8; wire::FILE_CHUNK];
    set_file(app, id, index, FileStatus::Transferring, 0);
    loop {
        if cancel.is_cancelled() {
            let _ = link.send_sharing(&wire::cancel()).await;
            link.send(&wire::disconnection()).await.ok();
            bail!("Cancelled");
        }
        check_peer(link)?;
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if offset + read as i64 > total {
            bail!("{} changed while sending", path.display());
        }
        let frame = wire::payload(
            payload_id,
            PayloadType::File,
            total,
            offset,
            buffer[..read].to_vec(),
            false,
        );
        link.send(&frame).await?;
        offset += read as i64;
        set_file(app, id, index, FileStatus::Transferring, offset as u64);
    }
    if offset != total {
        bail!("{} changed while sending", path.display());
    }
    link.send(&wire::payload(
        payload_id,
        PayloadType::File,
        total,
        offset,
        Vec::new(),
        true,
    ))
    .await?;
    set_file(app, id, index, FileStatus::Done, size);
    Ok(())
}

fn set_file(app: &App, id: &str, index: usize, status: FileStatus, done: u64) {
    let mut transfers = app.transfers.lock().unwrap();
    if let Some(f) = transfers.get_mut(id).and_then(|t| t.files.get_mut(index)) {
        f.status = status;
        f.done = done;
    }
    transfers.emit(id, status != FileStatus::Transferring);
}
