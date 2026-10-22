//! Receiving from Android's built-in Quick Share: the phone connects to us.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use prost::Message;
use serde_json::{json, Value};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::crypto::{alert_message, ServerHandshake, Session};
use super::link::{Event, Link};
use super::proto::location::nearby::connections::{
    connection_response_frame::ResponseStatus, v1_frame::FrameType,
};
use super::proto::sharing::nearby::{
    self as sharing, connection_response_frame::Status as Response,
    v1_frame::FrameType as SharingType, IntroductionFrame, WifiCredentials,
};
use super::wire::{self, EndpointInfo, Incoming};
use crate::app::App;
use crate::net;
use crate::protocol::FileDto;
use crate::storage;
use crate::transfers::{now, Direction, FileState, Status, Transfer};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_TIMEOUT: Duration = Duration::from_secs(30);

/// The device on the other end, as the UI shows it.
pub struct Sender {
    pub ip: IpAddr,
    pub info: EndpointInfo,
    pub pin: String,
}

impl Sender {
    pub fn id(&self) -> String {
        format!("quickshare:{}", self.ip)
    }

    pub fn name(&self) -> String {
        self.info
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Android device".into())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id(),
            "alias": self.name(),
            "deviceModel": "Quick Share",
            "deviceType": self.info.kind.as_device_type(),
            "ip": self.ip.to_string(),
            "kind": "quickshare",
            "pin": self.pin,
        })
    }
}

pub async fn handle(app: Arc<App>, stream: TcpStream) -> Result<()> {
    let ip = net::canonical(stream.peer_addr()?.ip());
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let (info, session) = timeout(HANDSHAKE_TIMEOUT, handshake(&mut reader, &mut writer))
        .await
        .map_err(|_| anyhow!("handshake timed out"))??;
    let sender = Sender {
        ip,
        info,
        pin: session.pin.clone(),
    };
    tracing::info!(
        "Quick Share connection from {} ({ip}), PIN {}",
        sender.name(),
        sender.pin
    );
    let mut link = Link::start(reader, writer, session);
    let intro = timeout(SETUP_TIMEOUT, async {
        pair(&mut link).await?;
        link.expect(SharingType::Introduction).await
    })
    .await
    .map_err(|_| anyhow!("timed out before the transfer started"))??
    .v1
    .and_then(|v1| v1.introduction)
    .ok_or_else(|| anyhow!("empty introduction"))?;

    // Every share reaches the user (a prompt or a message): not too many.
    if !app.requests.hit(ip) {
        let _ = link.send_sharing(&wire::response(Response::Reject)).await;
        link.close().await;
        bail!("too many requests from {ip}");
    }
    if !intro.file_metadata.is_empty() {
        receive_files(&app, link, &sender, intro).await
    } else if !intro.text_metadata.is_empty() || !intro.wifi_credentials_metadata.is_empty() {
        receive_text(&app, link, &sender, intro).await
    } else {
        link.send_sharing(&wire::response(Response::UnsupportedAttachmentType))
            .await?;
        link.close().await;
        bail!("nothing we can receive (apps or streams)")
    }
}

/// Plain-text part: connection request, UKEY2, connection responses.
async fn handshake(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
) -> Result<(EndpointInfo, Session)> {
    let (kind, v1) = wire::decode_offline(&wire::read_frame(reader).await?)?;
    if kind != FrameType::ConnectionRequest {
        bail!("expected a connection request, got {kind:?}");
    }
    let info = v1
        .connection_request
        .and_then(|r| r.endpoint_info)
        .and_then(|bytes| EndpointInfo::decode(&bytes))
        .context("connection request without endpoint info")?;

    let client_init = wire::read_frame(reader).await?;
    let (handshake, server_init) = match ServerHandshake::new(&client_init) {
        Ok(step) => step,
        Err(err) => {
            let _ = wire::write_frame(writer, &alert_message(&err)).await;
            return Err(err.into());
        }
    };
    wire::write_frame(writer, &server_init).await?;
    let client_finish = wire::read_frame(reader).await?;
    let session = match handshake.finish(&client_finish) {
        Ok(session) => session,
        Err(err) => {
            let _ = wire::write_frame(writer, &alert_message(&err)).await;
            return Err(err.into());
        }
    };

    let (kind, v1) = wire::decode_offline(&wire::read_frame(reader).await?)?;
    let accepted =
        v1.connection_response.and_then(|r| r.response) == Some(ResponseStatus::Accept as i32);
    if kind != FrameType::ConnectionResponse || !accepted {
        bail!("the other device did not confirm the connection");
    }
    wire::write_frame(writer, &wire::connection_response().encode_to_vec()).await?;
    Ok((info, session))
}

/// The "paired key" exchange: meaningless without Google accounts, but both
/// sides must go through it. Same steps for sender and receiver.
pub async fn pair(link: &mut Link) -> Result<()> {
    link.send_sharing(&wire::paired_key_encryption()).await?;
    link.expect(SharingType::PairedKeyEncryption).await?;
    link.send_sharing(&wire::paired_key_result()).await?;
    link.expect(SharingType::PairedKeyResult).await?;
    Ok(())
}

fn is_cancel(data: &[u8]) -> bool {
    sharing::Frame::decode(data).is_ok_and(|f| wire::sharing_type(&f) == SharingType::Cancel)
}

fn file_dto(meta: &sharing::FileMetadata) -> FileDto {
    let name = meta.name.clone().unwrap_or_default();
    let name = if name.trim().is_empty() {
        "file".to_string()
    } else {
        name
    };
    // Shared folders keep their structure; save_stream sanitizes the path.
    let file_name = match meta.parent_folder.as_deref().map(str::trim) {
        Some(folder) if !folder.is_empty() => format!("{folder}/{name}"),
        _ => name,
    };
    let file_type = meta
        .mime_type
        .clone()
        .filter(|m| !m.is_empty() && m != "application/octet-stream")
        .unwrap_or_else(|| {
            mime_guess::from_path(&file_name)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });
    FileDto {
        id: meta.payload_id.unwrap_or_default().to_string(),
        file_name,
        size: meta.size.unwrap_or_default().max(0) as u64,
        file_type,
        sha256: None,
        preview: None,
        metadata: None,
    }
}

/// Most files one Quick Share request may offer.
const MAX_FILES: usize = 1000;
/// Most text parts (texts, links, Wi‑Fi networks) in one share.
const MAX_TEXTS: usize = 8;

async fn receive_files(
    app: &Arc<App>,
    mut link: Link,
    sender: &Sender,
    intro: IntroductionFrame,
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if intro.file_metadata.len() > MAX_FILES
        || !intro
            .file_metadata
            .iter()
            .all(|m| seen.insert(m.payload_id.unwrap_or_default()))
    {
        let _ = link.send_sharing(&wire::response(Response::Reject)).await;
        link.close().await;
        bail!("too many or duplicate files offered");
    }
    let dtos: Vec<FileDto> = intro.file_metadata.iter().map(file_dto).collect();
    let mut transfer = Transfer::new(
        Direction::In,
        &sender.id(),
        sender.to_json(),
        dtos.iter()
            .cloned()
            .map(|d| FileState::new(d, None))
            .collect(),
    );
    transfer.peer_ip = Some(sender.ip);
    let total = transfer.total();
    let id = app.transfers.lock().unwrap().add(transfer);

    let download_dir = app.settings().download_dir.clone();
    let _ = std::fs::create_dir_all(&download_dir);
    if storage::free_space(&download_dir).is_some_and(|free| free < total) {
        app.finish(&id, Status::Failed, Some("Not enough free space".into()));
        link.send_sharing(&wire::response(Response::NotEnoughSpace))
            .await?;
        link.close().await;
        return Ok(());
    }

    // Ask, while watching the connection: the phone may give up meanwhile.
    let answer = {
        let ask = app.ask_accept(&id);
        tokio::pin!(ask);
        loop {
            tokio::select! {
                accepted = &mut ask => break Some(accepted),
                event = link.next() => match event {
                    Event::Closed(_) => break None,
                    Event::Payload(Incoming::Bytes { data, .. }) if is_cancel(&data) => break None,
                    Event::Payload(_) => {}
                },
            }
        }
    };
    let still_open = app
        .transfers
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|t| !t.status.is_final());
    match answer {
        None => {
            app.finish(
                &id,
                Status::Cancelled,
                Some("Cancelled by the sender".into()),
            );
            return Ok(());
        }
        Some(false) | Some(true) if !still_open => {
            let _ = link.send_sharing(&wire::response(Response::Reject)).await;
            link.close().await;
            return Ok(());
        }
        Some(false) => {
            app.finish(&id, Status::Declined, None);
            let _ = link.send_sharing(&wire::response(Response::Reject)).await;
            link.close().await;
            return Ok(());
        }
        Some(true) => {}
    }

    {
        let mut transfers = app.transfers.lock().unwrap();
        if let Some(t) = transfers.get_mut(&id) {
            t.status = Status::Transferring;
            t.started = Some(now());
        }
        transfers.emit(&id, true);
    }
    let cancel = CancellationToken::new();
    app.register_cancel(&id, cancel.clone());
    link.send_sharing(&wire::response(Response::Accept)).await?;

    // One writer task per file, started when its data begins to arrive and
    // fed through a channel; the storage code does the .part file, progress,
    // renaming and the "received" notification.
    struct Sink {
        tx: mpsc::Sender<std::io::Result<Bytes>>,
        offset: i64,
        size: i64,
    }
    let mut waiting: HashMap<i64, FileDto> = intro
        .file_metadata
        .iter()
        .map(|m| m.payload_id.unwrap_or_default())
        .zip(dtos)
        .collect();
    let mut sinks: HashMap<i64, Sink> = HashMap::new();
    let mut writers = Vec::new();

    let outcome: Option<(Status, String)> = loop {
        if sinks.is_empty() && waiting.is_empty() {
            break None;
        }
        let event = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = link.send_sharing(&wire::cancel()).await;
                break Some((Status::Cancelled, "Cancelled".into()));
            }
            event = link.next() => event,
        };
        match event {
            Event::Payload(Incoming::FileChunk {
                id: payload,
                offset,
                body,
                last,
            }) => {
                if let Some(dto) = waiting.remove(&payload) {
                    let (tx, mut rx) = mpsc::channel::<std::io::Result<Bytes>>(8);
                    let stream = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx));
                    let size = dto.size as i64;
                    let (app, id) = (app.clone(), id.clone());
                    writers.push(tokio::spawn(async move {
                        if let Err(err) = storage::save_stream(&app, &id, &dto, stream).await {
                            tracing::warn!("Quick Share: could not save {}: {err}", dto.file_name);
                        }
                    }));
                    sinks.insert(
                        payload,
                        Sink {
                            tx,
                            offset: 0,
                            size,
                        },
                    );
                }
                let Some(sink) = sinks.get_mut(&payload) else {
                    continue; // a file we already gave up on
                };
                // In order, and never more than was announced and accepted.
                if offset != sink.offset || sink.offset + body.len() as i64 > sink.size {
                    break Some((Status::Failed, "Corrupted transfer".into()));
                }
                if !body.is_empty() {
                    sink.offset += body.len() as i64;
                    if sink.tx.send(Ok(Bytes::from(body))).await.is_err() {
                        let _ = link.send_sharing(&wire::cancel()).await;
                        break Some((Status::Failed, "Could not save the file".into()));
                    }
                }
                if last {
                    sinks.remove(&payload); // closing the channel completes the file
                }
            }
            Event::Payload(Incoming::Bytes { data, .. }) => {
                if is_cancel(&data) {
                    break Some((Status::Cancelled, "Cancelled by the sender".into()));
                }
            }
            Event::Closed(reason) => {
                let reason = reason.unwrap_or_else(|| "The other device hung up".into());
                break Some((Status::Failed, reason));
            }
        }
    };
    if let Some((status, message)) = outcome {
        app.finish(&id, status, Some(message));
    }
    drop(sinks);
    for writer in writers {
        let _ = writer.await;
    }
    app.unregister_cancel(&id);
    link.close().await;
    Ok(())
}

async fn receive_text(
    app: &Arc<App>,
    mut link: Link,
    sender: &Sender,
    intro: IntroductionFrame,
) -> Result<()> {
    enum Part {
        Text,
        Wifi(String),
    }
    if intro.text_metadata.len() + intro.wifi_credentials_metadata.len() > MAX_TEXTS {
        let _ = link.send_sharing(&wire::response(Response::Reject)).await;
        link.close().await;
        bail!("too many texts offered");
    }
    let mut wanted: HashMap<i64, Part> = HashMap::new();
    // Plain text, links, addresses and phone numbers are all shown as text.
    for meta in &intro.text_metadata {
        wanted.insert(meta.payload_id.unwrap_or_default(), Part::Text);
    }
    for meta in &intro.wifi_credentials_metadata {
        let ssid = meta.ssid.clone().unwrap_or_default();
        wanted.insert(meta.payload_id.unwrap_or_default(), Part::Wifi(ssid));
    }
    // Text is shown, not saved, so it needs no confirmation (as with LocalSend).
    link.send_sharing(&wire::response(Response::Accept)).await?;

    let mut parts = Vec::new();
    let mut length = 0;
    let mut wifi = false;
    // One deadline for the whole exchange, however chatty the sender.
    let deadline = tokio::time::Instant::now() + SETUP_TIMEOUT;
    while !wanted.is_empty() {
        match tokio::time::timeout_at(deadline, link.next()).await {
            Err(_) => bail!("timed out waiting for the text"),
            Ok(Event::Payload(Incoming::Bytes { id, data })) => match wanted.remove(&id) {
                Some(_) if length + data.len() > crate::protocol::MAX_TEXT => {
                    bail!("text too long")
                }
                Some(Part::Text) => {
                    length += data.len();
                    parts.push(String::from_utf8_lossy(&data).into_owned())
                }
                Some(Part::Wifi(ssid)) => {
                    length += data.len();
                    wifi = true;
                    let password = WifiCredentials::decode(data.as_slice())
                        .ok()
                        .and_then(|c| c.password)
                        .unwrap_or_default();
                    parts.push(format!("Wi-Fi network “{ssid}”\nPassword: {password}"));
                }
                None if is_cancel(&data) => bail!("cancelled by the sender"),
                None => bail!("unexpected data"),
            },
            Ok(Event::Payload(Incoming::FileChunk { .. })) => bail!("unexpected file data"),
            Ok(Event::Closed(reason)) => {
                bail!(
                    "{}",
                    reason.unwrap_or_else(|| "the other device hung up".into())
                )
            }
        }
    }
    link.close().await;

    let text = parts.join("\n\n");
    let dto = FileDto::for_text(&text);
    let mut transfer = Transfer::new(
        Direction::In,
        &sender.id(),
        sender.to_json(),
        vec![FileState::new(dto, None)],
    );
    transfer.peer_ip = Some(sender.ip);
    transfer.message = Some(text);
    // A Wi‑Fi password is shown, but not kept in the history file.
    transfer.sensitive = wifi;
    transfer.started = Some(now());
    for f in transfer.files.iter_mut() {
        f.status = crate::transfers::FileStatus::Done;
        f.done = f.size;
    }
    let id = app.transfers.lock().unwrap().add(transfer);
    app.finish(&id, Status::Done, None);
    app.on_text_received(&id);
    Ok(())
}
