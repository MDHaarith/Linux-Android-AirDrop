//! Quick Share wire format: length-prefixed frames, the "endpoint info"
//! blob, the mDNS service name and builders for the frames we send.

use std::collections::HashMap;

use anyhow::{bail, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use prost::Message;
use rand::{Rng, RngCore};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::proto::location::nearby::connections::{
    self as connections, connection_request_frame, connection_response_frame, offline_frame,
    os_info, payload_transfer_frame, v1_frame, OfflineFrame, PayloadTransferFrame,
};
use super::proto::sharing::nearby::{
    self as sharing, connection_response_frame as response, frame, paired_key_result_frame,
};

/// Android drops connections whose frames are larger than this.
pub const MAX_FRAME: usize = 5 * 1024 * 1024;

/// Bytes of a file per payload chunk (what Android uses too).
pub const FILE_CHUNK: usize = 512 * 1024;

/// mDNS service type: "_" + the first 6 bytes of SHA256("NearbySharing") + "._tcp".
pub const SERVICE_TYPE: &str = "_FC9F5ED42C8A._tcp.local.";

const PCP: u8 = 0x23;
const SERVICE_ID_HASH: [u8; 3] = [0xFC, 0x9F, 0x5E];

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Vec<u8>> {
    let length = reader.read_u32().await? as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame of {length} bytes is too large"),
        ));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame).await?;
    Ok(frame)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &[u8],
) -> std::io::Result<()> {
    writer.write_u32(frame.len() as u32).await?;
    writer.write_all(frame).await?;
    writer.flush().await
}

/// Four random alphanumeric characters; identifies us for one session.
pub fn random_endpoint_id() -> [u8; 4] {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut rng = rand::thread_rng();
    std::array::from_fn(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
}

/// The mDNS instance name: base64url(PCP ‖ endpoint id ‖ service id hash ‖ 0 0).
pub fn service_name(endpoint_id: &[u8; 4]) -> String {
    let mut bytes = vec![PCP];
    bytes.extend_from_slice(endpoint_id);
    bytes.extend_from_slice(&SERVICE_ID_HASH);
    bytes.extend_from_slice(&[0, 0]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The endpoint id inside a Quick Share mDNS instance name.
pub fn parse_service_name(name: &str) -> Option<[u8; 4]> {
    let bytes = decode_base64(name)?;
    if bytes.len() < 10 || bytes[0] != PCP || bytes[5..8] != SERVICE_ID_HASH {
        return None;
    }
    bytes[1..5].try_into().ok()
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Unknown = 0,
    Phone = 1,
    Tablet = 2,
    Laptop = 3,
}

impl DeviceKind {
    fn from_bits(bits: u8) -> Self {
        match bits {
            1 | 5 => DeviceKind::Phone, // 5 = foldable
            2 => DeviceKind::Tablet,
            3 => DeviceKind::Laptop,
            _ => DeviceKind::Unknown,
        }
    }

    /// The LocalSend device type the UI already knows how to draw.
    pub fn as_device_type(self) -> &'static str {
        match self {
            DeviceKind::Phone | DeviceKind::Tablet => "mobile",
            DeviceKind::Laptop => "desktop",
            DeviceKind::Unknown => "headless",
        }
    }
}

/// "Endpoint info": who a device is, in the mDNS TXT record and the
/// connection request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointInfo {
    pub kind: DeviceKind,
    /// Absent when the device hides its name (contacts-only visibility).
    pub name: Option<String>,
}

impl EndpointInfo {
    pub fn encode(&self) -> Vec<u8> {
        // version (3 bits) | hidden (1 bit) | device type (3 bits) | reserved (1 bit)
        let hidden = self.name.is_none();
        let mut out = vec![((hidden as u8) << 4) | ((self.kind as u8) << 1)];
        // Salt and encrypted metadata key: only meaningful to Google's
        // contacts-based visibility, random for "everyone".
        let mut opaque = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut opaque);
        out.extend_from_slice(&opaque);
        if let Some(name) = &self.name {
            let mut end = name.len().min(255);
            while !name.is_char_boundary(end) {
                end -= 1;
            }
            out.push(end as u8);
            out.extend_from_slice(&name.as_bytes()[..end]);
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 17 {
            return None;
        }
        let kind = DeviceKind::from_bits((bytes[0] >> 1) & 0b111);
        let hidden = bytes[0] & 0x10 != 0;
        let name = if hidden {
            None
        } else {
            let length = *bytes.get(17)? as usize;
            let raw = bytes.get(18..18 + length)?;
            Some(String::from_utf8_lossy(raw).into_owned())
        };
        Some(Self { kind, name })
    }

    pub fn to_base64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.encode())
    }

    pub fn from_base64(text: &str) -> Option<Self> {
        Self::decode(&decode_base64(text)?)
    }
}

// ------------------------------------------------------------ offline frames

fn offline(
    kind: v1_frame::FrameType,
    fill: impl FnOnce(&mut connections::V1Frame),
) -> OfflineFrame {
    let mut v1 = connections::V1Frame {
        r#type: Some(kind as i32),
        ..Default::default()
    };
    fill(&mut v1);
    OfflineFrame {
        version: Some(offline_frame::Version::V1 as i32),
        v1: Some(v1),
    }
}

pub fn connection_request(endpoint_id: &[u8; 4], info: &EndpointInfo) -> OfflineFrame {
    offline(v1_frame::FrameType::ConnectionRequest, |v1| {
        v1.connection_request = Some(connections::ConnectionRequestFrame {
            endpoint_id: Some(String::from_utf8_lossy(endpoint_id).into_owned()),
            endpoint_name: info.name.clone(),
            endpoint_info: Some(info.encode()),
            mediums: vec![connection_request_frame::Medium::WifiLan as i32],
            ..Default::default()
        });
    })
}

// `status` is deprecated in favor of `response`; older peers still read it.
#[allow(deprecated)]
pub fn connection_response() -> OfflineFrame {
    offline(v1_frame::FrameType::ConnectionResponse, |v1| {
        v1.connection_response = Some(connections::ConnectionResponseFrame {
            status: Some(0),
            response: Some(connection_response_frame::ResponseStatus::Accept as i32),
            os_info: Some(connections::OsInfo {
                r#type: Some(os_info::OsType::Linux as i32),
            }),
            ..Default::default()
        });
    })
}

pub fn keep_alive(ack: bool) -> OfflineFrame {
    offline(v1_frame::FrameType::KeepAlive, |v1| {
        v1.keep_alive = Some(connections::KeepAliveFrame {
            ack: Some(ack),
            ..Default::default()
        });
    })
}

pub fn disconnection() -> OfflineFrame {
    offline(v1_frame::FrameType::Disconnection, |v1| {
        v1.disconnection = Some(Default::default());
    })
}

pub fn payload(
    id: i64,
    kind: payload_transfer_frame::payload_header::PayloadType,
    total: i64,
    offset: i64,
    body: Vec<u8>,
    last: bool,
) -> OfflineFrame {
    offline(v1_frame::FrameType::PayloadTransfer, |v1| {
        v1.payload_transfer = Some(PayloadTransferFrame {
            packet_type: Some(payload_transfer_frame::PacketType::Data as i32),
            payload_header: Some(payload_transfer_frame::PayloadHeader {
                id: Some(id),
                r#type: Some(kind as i32),
                total_size: Some(total),
                is_sensitive: Some(false),
                ..Default::default()
            }),
            payload_chunk: Some(payload_transfer_frame::PayloadChunk {
                flags: Some(last as i32),
                offset: Some(offset),
                body: Some(body),
                ..Default::default()
            }),
            ..Default::default()
        });
    })
}

/// A bytes payload the way Android sends one: the data, then an empty last chunk.
pub fn bytes_payload(id: i64, data: Vec<u8>) -> [OfflineFrame; 2] {
    use payload_transfer_frame::payload_header::PayloadType;
    let size = data.len() as i64;
    [
        payload(id, PayloadType::Bytes, size, 0, data, false),
        payload(id, PayloadType::Bytes, size, size, Vec::new(), true),
    ]
}

pub fn random_payload_id() -> i64 {
    rand::thread_rng().gen()
}

// ------------------------------------------------------------ sharing frames

fn sharing_frame(
    kind: sharing::v1_frame::FrameType,
    fill: impl FnOnce(&mut sharing::V1Frame),
) -> sharing::Frame {
    let mut v1 = sharing::V1Frame {
        r#type: Some(kind as i32),
        ..Default::default()
    };
    fill(&mut v1);
    sharing::Frame {
        version: Some(frame::Version::V1 as i32),
        v1: Some(v1),
    }
}

/// Without Google account certificates there is nothing real to put here;
/// random bytes of the usual sizes are accepted ("everyone" visibility).
pub fn paired_key_encryption() -> sharing::Frame {
    let mut secret_id_hash = vec![0u8; 6];
    let mut signed_data = vec![0u8; 72];
    rand::thread_rng().fill_bytes(&mut secret_id_hash);
    rand::thread_rng().fill_bytes(&mut signed_data);
    sharing_frame(sharing::v1_frame::FrameType::PairedKeyEncryption, |v1| {
        v1.paired_key_encryption = Some(sharing::PairedKeyEncryptionFrame {
            secret_id_hash: Some(secret_id_hash),
            signed_data: Some(signed_data),
            ..Default::default()
        });
    })
}

pub fn paired_key_result() -> sharing::Frame {
    sharing_frame(sharing::v1_frame::FrameType::PairedKeyResult, |v1| {
        v1.paired_key_result = Some(sharing::PairedKeyResultFrame {
            status: Some(paired_key_result_frame::Status::Unable as i32),
            ..Default::default()
        });
    })
}

pub fn introduction(intro: sharing::IntroductionFrame) -> sharing::Frame {
    sharing_frame(sharing::v1_frame::FrameType::Introduction, |v1| {
        v1.introduction = Some(intro);
    })
}

pub fn response(status: response::Status) -> sharing::Frame {
    sharing_frame(sharing::v1_frame::FrameType::Response, |v1| {
        v1.connection_response = Some(sharing::ConnectionResponseFrame {
            status: Some(status as i32),
            ..Default::default()
        });
    })
}

pub fn cancel() -> sharing::Frame {
    sharing_frame(sharing::v1_frame::FrameType::Cancel, |_| {})
}

/// The sharing frame type, or `UnknownFrameType`.
pub fn sharing_type(frame: &sharing::Frame) -> sharing::v1_frame::FrameType {
    frame
        .v1
        .as_ref()
        .and_then(|v1| v1.r#type)
        .and_then(|t| sharing::v1_frame::FrameType::try_from(t).ok())
        .unwrap_or(sharing::v1_frame::FrameType::UnknownFrameType)
}

// ------------------------------------------------------------ incoming payloads

/// Something the peer sent after the handshake.
#[derive(Debug)]
pub enum Incoming {
    /// A complete BYTES payload (a sharing frame or the text of a message).
    Bytes { id: i64, data: Vec<u8> },
    /// A piece of a FILE payload.
    FileChunk {
        id: i64,
        offset: i64,
        body: Vec<u8>,
        last: bool,
    },
}

/// Joins BYTES payload chunks; passes FILE chunks through.
#[derive(Default)]
pub struct Assembler {
    buffers: HashMap<i64, Vec<u8>>,
}

impl Assembler {
    pub fn push(&mut self, frame: PayloadTransferFrame) -> Result<Option<Incoming>> {
        use payload_transfer_frame::payload_header::PayloadType;
        let (Some(header), Some(chunk)) = (frame.payload_header, frame.payload_chunk) else {
            // Control messages (payload errors / cancels) carry no chunk.
            return Ok(None);
        };
        let id = header.id.unwrap_or_default();
        let offset = chunk.offset.unwrap_or_default();
        let last = chunk.flags.unwrap_or_default() & 1 != 0;
        let body = chunk.body.unwrap_or_default();
        match header.r#type.and_then(|t| PayloadType::try_from(t).ok()) {
            Some(PayloadType::Bytes) => {
                if header.total_size.unwrap_or_default() > MAX_FRAME as i64 {
                    bail!("bytes payload too large");
                }
                let buffer = self.buffers.entry(id).or_default();
                if offset != buffer.len() as i64 {
                    self.buffers.remove(&id);
                    bail!("unexpected offset {offset} in bytes payload");
                }
                buffer.extend_from_slice(&body);
                if buffer.len() > MAX_FRAME {
                    self.buffers.remove(&id);
                    bail!("bytes payload too large");
                }
                if last {
                    let data = self.buffers.remove(&id).unwrap_or_default();
                    return Ok(Some(Incoming::Bytes { id, data }));
                }
                Ok(None)
            }
            Some(PayloadType::File) => Ok(Some(Incoming::FileChunk {
                id,
                offset,
                body,
                last,
            })),
            _ => Ok(None),
        }
    }
}

pub fn decode_offline(bytes: &[u8]) -> Result<(v1_frame::FrameType, connections::V1Frame)> {
    let frame = OfflineFrame::decode(bytes)?;
    let v1 = frame.v1.unwrap_or_default();
    let kind = v1
        .r#type
        .and_then(|t| v1_frame::FrameType::try_from(t).ok())
        .unwrap_or(v1_frame::FrameType::UnknownFrameType);
    Ok((kind, v1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_round_trips() {
        let id = *b"Ab3x";
        let name = service_name(&id);
        assert_eq!(name.len(), 14);
        assert!(!name.contains('='));
        assert_eq!(parse_service_name(&name), Some(id));
        assert_eq!(parse_service_name("not-quick-share"), None);
    }

    #[test]
    fn endpoint_info_round_trips() {
        let info = EndpointInfo {
            kind: DeviceKind::Laptop,
            name: Some("Haarith's ThinkPad".into()),
        };
        let encoded = info.encode();
        assert_eq!(encoded[0], 3 << 1);
        assert_eq!(encoded[17] as usize, "Haarith's ThinkPad".len());
        assert_eq!(EndpointInfo::decode(&encoded), Some(info.clone()));
        assert_eq!(EndpointInfo::from_base64(&info.to_base64()), Some(info));

        let hidden = EndpointInfo {
            kind: DeviceKind::Phone,
            name: None,
        };
        assert_eq!(EndpointInfo::decode(&hidden.encode()), Some(hidden));
        assert_eq!(EndpointInfo::decode(&[0; 5]), None);
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let info = EndpointInfo {
            kind: DeviceKind::Laptop,
            name: Some("é".repeat(200)),
        };
        let decoded = EndpointInfo::decode(&info.encode()).unwrap();
        assert_eq!(decoded.name.unwrap().chars().count(), 127);
    }

    #[test]
    fn bytes_payloads_are_reassembled() {
        let mut assembler = Assembler::default();
        let frames = bytes_payload(42, b"hello".to_vec());
        let [first, last] = frames.map(|f| f.v1.unwrap().payload_transfer.unwrap());
        assert!(assembler.push(first).unwrap().is_none());
        match assembler.push(last).unwrap() {
            Some(Incoming::Bytes { id, data }) => {
                assert_eq!(id, 42);
                assert_eq!(data, b"hello");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn frames_are_length_prefixed() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"abc").await.unwrap();
        assert_eq!(buffer, [0, 0, 0, 3, b'a', b'b', b'c']);
        let mut reader = buffer.as_slice();
        assert_eq!(read_frame(&mut reader).await.unwrap(), b"abc");
        let mut huge = ((MAX_FRAME + 1) as u32).to_be_bytes().as_slice().to_vec();
        huge.extend_from_slice(&[0; 8]);
        assert!(read_frame(&mut huge.as_slice()).await.is_err());
    }
}
