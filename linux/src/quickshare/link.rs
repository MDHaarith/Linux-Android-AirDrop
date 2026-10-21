//! A Quick Share connection after the handshake. A writer task seals and
//! sends frames (plus keep-alives); a reader task opens frames, answers
//! keep-alives and hands payloads to the protocol logic. Talking to the link
//! only goes through channels, so callers can `select!` on it freely.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use prost::Message;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::crypto::{Opener, Sealer, Session};
use super::proto::location::nearby::connections::{v1_frame::FrameType, OfflineFrame};
use super::proto::sharing::nearby as sharing;
use super::wire::{self, Assembler, Incoming};

/// Android sends keep-alives every 10 seconds and expects the same from us.
const KEEP_ALIVE: Duration = Duration::from_secs(10);
/// Silence after which the peer is considered gone.
const PEER_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug)]
pub enum Event {
    Payload(Incoming),
    /// The peer hung up (`None`) or the connection broke.
    Closed(Option<String>),
}

pub struct Link {
    out: Option<mpsc::Sender<Vec<u8>>>,
    events: mpsc::Receiver<Event>,
    reader: JoinHandle<()>,
    writer: Option<JoinHandle<()>>,
}

impl Link {
    pub fn start(reader: OwnedReadHalf, writer: OwnedWriteHalf, session: Session) -> Self {
        let (out, out_rx) = mpsc::channel(4);
        let (events_tx, events) = mpsc::channel(16);
        let writer = tokio::spawn(write_loop(writer, session.sealer, out_rx));
        let reader = tokio::spawn(read_loop(reader, session.opener, out.clone(), events_tx));
        Self {
            out: Some(out),
            events,
            reader,
            writer: Some(writer),
        }
    }

    pub async fn send(&self, frame: &OfflineFrame) -> Result<()> {
        let out = self
            .out
            .as_ref()
            .ok_or_else(|| anyhow!("connection closed"))?;
        out.send(frame.encode_to_vec())
            .await
            .map_err(|_| anyhow!("connection closed"))
    }

    /// Sharing frames travel as BYTES payloads.
    pub async fn send_sharing(&self, frame: &sharing::Frame) -> Result<()> {
        for part in wire::bytes_payload(wire::random_payload_id(), frame.encode_to_vec()) {
            self.send(&part).await?;
        }
        Ok(())
    }

    pub async fn next(&mut self) -> Event {
        self.events
            .recv()
            .await
            .unwrap_or(Event::Closed(Some("connection closed".into())))
    }

    /// An event that already arrived, without waiting.
    pub fn try_next(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// The next sharing frame of the given type, skipping informational ones.
    pub async fn expect(&mut self, wanted: sharing::v1_frame::FrameType) -> Result<sharing::Frame> {
        use sharing::v1_frame::FrameType as Kind;
        loop {
            match self.next().await {
                Event::Payload(Incoming::Bytes { data, .. }) => {
                    let Ok(frame) = sharing::Frame::decode(data.as_slice()) else {
                        continue;
                    };
                    match wire::sharing_type(&frame) {
                        kind if kind == wanted => return Ok(frame),
                        Kind::Cancel => bail!("cancelled by the other device"),
                        Kind::ProgressUpdate | Kind::CertificateInfo | Kind::UnknownFrameType => {}
                        other => bail!("expected {wanted:?}, got {other:?}"),
                    }
                }
                Event::Payload(Incoming::FileChunk { .. }) => bail!("unexpected file data"),
                Event::Closed(reason) => {
                    bail!(
                        "{}",
                        reason.unwrap_or_else(|| "the other device hung up".into())
                    )
                }
            }
        }
    }

    /// Says goodbye and waits briefly for the peer to hang up, so it reads
    /// everything we sent before the socket goes away.
    pub async fn close(mut self) {
        let _ = self.send(&wire::disconnection()).await;
        self.out = None; // the writer flushes, then shuts the socket down
        if let Some(writer) = self.writer.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(self.next().await, Event::Closed(_)) {}
        })
        .await;
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some(writer) = &self.writer {
            writer.abort();
        }
    }
}

async fn write_loop(
    mut writer: OwnedWriteHalf,
    mut sealer: Sealer,
    mut frames: mpsc::Receiver<Vec<u8>>,
) {
    let start = tokio::time::Instant::now() + KEEP_ALIVE;
    let mut keep_alive = tokio::time::interval_at(start, KEEP_ALIVE);
    loop {
        let frame = tokio::select! {
            frame = frames.recv() => match frame {
                Some(frame) => frame,
                None => break,
            },
            _ = keep_alive.tick() => wire::keep_alive(false).encode_to_vec(),
        };
        if let Err(err) = wire::write_frame(&mut writer, &sealer.seal(&frame)).await {
            tracing::debug!("Quick Share: write failed: {err}");
            return;
        }
    }
    let _ = writer.shutdown().await;
}

async fn read_loop(
    mut reader: OwnedReadHalf,
    mut opener: Opener,
    out: mpsc::Sender<Vec<u8>>,
    events: mpsc::Sender<Event>,
) {
    let mut assembler = Assembler::default();
    let reason = loop {
        let raw = match tokio::time::timeout(PEER_TIMEOUT, wire::read_frame(&mut reader)).await {
            Err(_) => break Some("the other device stopped responding".to_string()),
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => break None,
            Ok(Err(err)) => break Some(err.to_string()),
            Ok(Ok(raw)) => raw,
        };
        let (kind, v1) = match opener
            .open(&raw)
            .and_then(|plain| wire::decode_offline(&plain))
        {
            Ok(frame) => frame,
            Err(err) => break Some(format!("invalid frame: {err}")),
        };
        match kind {
            FrameType::KeepAlive => {
                if v1.keep_alive.and_then(|k| k.ack) != Some(true) {
                    // Best effort: our own keep-alives cover a full queue.
                    let _ = out.try_send(wire::keep_alive(true).encode_to_vec());
                }
            }
            FrameType::Disconnection => break None,
            FrameType::PayloadTransfer => {
                match assembler.push(v1.payload_transfer.unwrap_or_default()) {
                    Ok(Some(payload)) => {
                        if events.send(Event::Payload(payload)).await.is_err() {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(err) => break Some(err.to_string()),
                }
            }
            other => tracing::debug!("Quick Share: ignoring {other:?} frame"),
        }
    };
    let _ = events.send(Event::Closed(reason)).await;
}
