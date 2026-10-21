//! Talks to the running service from the edge window. GTK owns the main
//! thread, so HTTP runs on a small tokio thread; replies come back through a
//! channel the GTK side polls.

use std::sync::mpsc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};

use super::drop::Payload;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub alias: String,
    pub device_type: String,
    pub kind: String,
}

impl Device {
    pub fn subtitle(&self) -> &str {
        match self.kind.as_str() {
            "quickshare" => "Quick Share",
            "web" => "Browser",
            _ if self.device_type == "mobile" => "Phone",
            _ => "Computer",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub status: String,
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
}

pub enum Command {
    Devices,
    Wake,
    Send { peer: String, payload: Payload },
    Progress(String),
}

pub enum Reply {
    Devices(Vec<Device>),
    /// The service is not running.
    Offline,
    Sent(Result<String, String>),
    Progress(String, Progress),
}

pub struct Api {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    pub replies: mpsc::Receiver<Reply>,
}

impl Api {
    pub fn start() -> Self {
        let (commands, mut rx) = tokio::sync::mpsc::unbounded_channel::<Command>();
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async move {
                while let Some(command) = rx.recv().await {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        if let Some(reply) = handle(command).await {
                            let _ = tx.send(reply);
                        }
                    });
                }
            });
        });
        Self { commands, replies }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

struct Instance {
    port: u16,
    token: String,
}

fn instance() -> Option<Instance> {
    let text = std::fs::read(crate::config::runtime_dir().join("instance.json")).ok()?;
    let value: Value = serde_json::from_slice(&text).ok()?;
    Some(Instance {
        port: value.get("port")?.as_u64()? as u16,
        token: value.get("token")?.as_str()?.to_string(),
    })
}

async fn call(
    method: Method,
    path: &str,
    content_type: &str,
    headers: &[(&str, String)],
    body: Bytes,
) -> Result<(u16, Value), String> {
    let instance = instance().ok_or("AirBridge is not running")?;
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", instance.port))
        .await
        .map_err(|_| "AirBridge is not running".to_string())?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(connection);
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{}", instance.port))
        .header("x-airbridge-token", &instance.token)
        .header("content-type", content_type);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let request = request.body(Full::new(body)).map_err(|e| e.to_string())?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn json_call(path: &str, body: Value) -> Result<Value, String> {
    let (status, value) = call(
        Method::POST,
        path,
        "application/json",
        &[],
        Bytes::from(body.to_string()),
    )
    .await?;
    if (200..300).contains(&status) {
        Ok(value)
    } else {
        Err(value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Could not send")
            .to_string())
    }
}

fn transfer_id(value: Value) -> Result<String, String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Unexpected answer from AirBridge".to_string())
}

async fn send(peer: String, payload: Payload) -> Result<String, String> {
    match payload {
        Payload::Paths(paths) => {
            transfer_id(json_call("/api/send", json!({ "peer": peer, "paths": paths })).await?)
        }
        Payload::Text(text) => {
            transfer_id(json_call("/api/send-text", json!({ "peer": peer, "text": text })).await?)
        }
        Payload::Url { url, name } => transfer_id(
            json_call(
                "/api/send-url",
                json!({ "peer": peer, "url": url, "name": name }),
            )
            .await?,
        ),
        Payload::Image { name, data } => {
            let encoded: String = name
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            let (status, value) = call(
                Method::POST,
                "/api/stage",
                "application/octet-stream",
                &[("x-file-name", encoded)],
                Bytes::from(data),
            )
            .await?;
            if status != 200 {
                return Err("Could not prepare the image".into());
            }
            let staged = value
                .get("id")
                .and_then(Value::as_str)
                .ok_or("Could not prepare the image")?
                .to_string();
            transfer_id(json_call("/api/send", json!({ "peer": peer, "staged": [staged] })).await?)
        }
    }
}

pub fn parse_devices(state: &Value) -> Vec<Device> {
    state
        .get("peers")
        .and_then(Value::as_array)
        .map(|peers| {
            peers
                .iter()
                .filter_map(|p| {
                    Some(Device {
                        id: p.get("id")?.as_str()?.to_string(),
                        alias: p.get("alias")?.as_str()?.to_string(),
                        device_type: p
                            .get("deviceType")
                            .and_then(Value::as_str)
                            .unwrap_or("desktop")
                            .to_string(),
                        kind: p
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("localsend")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn handle(command: Command) -> Option<Reply> {
    match command {
        Command::Devices => Some(
            match call(
                Method::GET,
                "/api/state",
                "application/json",
                &[],
                Bytes::new(),
            )
            .await
            {
                Ok((200, state)) => Reply::Devices(parse_devices(&state)),
                _ => Reply::Offline,
            },
        ),
        Command::Wake => {
            let _ = call(
                Method::POST,
                "/api/wake",
                "application/json",
                &[],
                Bytes::new(),
            )
            .await;
            None
        }
        Command::Send { peer, payload } => Some(Reply::Sent(send(peer, payload).await)),
        Command::Progress(id) => {
            let (status, t) = call(
                Method::GET,
                &format!("/api/transfers/{id}"),
                "application/json",
                &[],
                Bytes::new(),
            )
            .await
            .ok()?;
            if status != 200 {
                return None;
            }
            Some(Reply::Progress(
                id,
                Progress {
                    status: t
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    done: t.get("done").and_then(Value::as_u64).unwrap_or(0),
                    total: t.get("total").and_then(Value::as_u64).unwrap_or(0),
                    error: t.get("error").and_then(Value::as_str).map(str::to_string),
                },
            ))
        }
    }
}
