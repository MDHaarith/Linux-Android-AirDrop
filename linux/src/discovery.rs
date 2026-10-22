//! Device discovery: LocalSend multicast announcements + HTTP register.
//!
//! 1. We multicast an announcement (`announce: true`) to 224.0.0.167:53317.
//! 2. Every device that hears it answers with `POST /api/localsend/v2/register`
//!    to our HTTPS server, which adds it to the registry.
//! 3. When we hear someone else's announcement we register with them, pinning
//!    the fingerprint they announced, and add them once that succeeded.
//! 4. Networks without multicast fall back to probing every local IP.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyper::Method;
use serde_json::json;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::Semaphore;

use crate::app::App;
use crate::client::{self, Body, Options, Target};
use crate::net;
use crate::peers::PeerKind;
use crate::protocol::{is_localsend_id, DeviceInfo, MulticastMessage, API};

const ANNOUNCE_DELAYS_MS: [u64; 3] = [100, 500, 2000];
const REANNOUNCE_INTERVAL: Duration = Duration::from_secs(45);
const LIVENESS_INTERVAL: Duration = Duration::from_secs(12);
const STALE_AFTER: Duration = Duration::from_secs(25);
const MAX_FAILURES: u32 = 2;
/// Devices being confirmed at once (each costs an outgoing connection).
const MAX_PENDING: usize = 32;

pub struct Discovery {
    socket: Option<Arc<UdpSocket>>,
    send_lock: tokio::sync::Mutex<()>,
    group: Ipv4Addr,
    port: u16,
    pending: Mutex<std::collections::HashSet<String>>,
    pub scanning: AtomicBool,
}

impl Discovery {
    pub fn multicast_ok(&self) -> bool {
        self.socket.is_some()
    }

    fn bind(group: Ipv4Addr, port: u16) -> std::io::Result<UdpSocket> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).into())?;
        let mut joined = 0;
        for iface in net::interfaces() {
            match socket.join_multicast_v4(&group, &iface.ip) {
                Ok(()) => joined += 1,
                Err(err) => tracing::debug!("cannot join {group} on {}: {err}", iface.name),
            }
        }
        if joined == 0 {
            socket.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)?;
        }
        socket.set_multicast_ttl_v4(1)?;
        // Loopback lets two instances on one machine see each other (handy for testing).
        socket.set_multicast_loop_v4(true)?;
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket.into())
    }

    pub async fn start(app: Arc<App>) -> Arc<Discovery> {
        let (group, port) = {
            let settings = app.settings();
            (
                settings
                    .multicast_group
                    .parse()
                    .unwrap_or(Ipv4Addr::new(224, 0, 0, 167)),
                settings.multicast_port,
            )
        };
        let socket = match Self::bind(group, port) {
            Ok(socket) => Some(Arc::new(socket)),
            Err(err) => {
                tracing::warn!(
                    "Multicast unavailable ({err}); use \"Scan network\" to find devices"
                );
                None
            }
        };
        let discovery = Arc::new(Discovery {
            socket,
            send_lock: tokio::sync::Mutex::new(()),
            group,
            port,
            pending: Mutex::new(Default::default()),
            scanning: AtomicBool::new(false),
        });

        if let Some(socket) = discovery.socket.clone() {
            let (app, discovery) = (app.clone(), discovery.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match socket.recv_from(&mut buf).await {
                        Ok((len, from)) => discovery.on_datagram(&app, &buf[..len], from.ip()),
                        Err(err) => {
                            tracing::debug!("multicast receive error: {err}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            });
        }
        {
            let (app, discovery) = (app.clone(), discovery.clone());
            tokio::spawn(async move {
                loop {
                    discovery.announce_burst(&app).await;
                    tokio::time::sleep(REANNOUNCE_INTERVAL).await;
                }
            });
        }
        {
            let (app, discovery) = (app.clone(), discovery.clone());
            tokio::spawn(async move { discovery.liveness_loop(&app).await });
        }
        discovery
    }

    async fn send(&self, payload: &[u8]) {
        let Some(socket) = &self.socket else { return };
        let _guard = self.send_lock.lock().await;
        let target = SocketAddr::V4(SocketAddrV4::new(self.group, self.port));
        let interfaces = net::interfaces();
        if interfaces.is_empty() {
            let _ = socket.send_to(payload, target).await;
            return;
        }
        for iface in interfaces {
            if let Err(err) = SockRef::from(socket.as_ref()).set_multicast_if_v4(&iface.ip) {
                tracing::debug!("set_multicast_if {}: {err}", iface.name);
                continue;
            }
            if let Err(err) = socket.send_to(payload, target).await {
                tracing::debug!("announce on {} failed: {err}", iface.name);
            }
        }
    }

    pub async fn announce_burst(&self, app: &App) {
        let payload = app.me().announcement_json(true).to_string();
        for delay in ANNOUNCE_DELAYS_MS {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            self.send(payload.as_bytes()).await;
        }
    }

    fn on_datagram(self: &Arc<Self>, app: &Arc<App>, data: &[u8], ip: IpAddr) {
        let Ok(message) = serde_json::from_slice::<MulticastMessage>(data) else {
            return;
        };
        let announce = message.is_announcement();
        let Ok(info) = message.info.sanitized() else {
            return;
        };
        let me = app.me();
        if !is_localsend_id(&info.fingerprint)
            || info.fingerprint.eq_ignore_ascii_case(&me.fingerprint)
        {
            return;
        }
        let ip = net::canonical(ip);
        // Only devices on our own networks, on a normal port: a spoofed packet
        // must not make us connect to arbitrary hosts and services.
        if !net::is_local_network(ip) || info.port < 1024 {
            return;
        }
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= MAX_PENDING || !pending.insert(info.fingerprint.clone()) {
                return;
            }
        }
        // Announcements and UDP answers (older LocalSend versions) alike are
        // confirmed over TCP: over HTTPS the certificate proves the id, so a
        // packet alone never adds or changes a device.
        let (app, discovery) = (app.clone(), self.clone());
        tokio::spawn(async move {
            let fingerprint = info.fingerprint.clone();
            let confirmed =
                register_with(&app, ip, info.port, info.is_https(), Some(&fingerprint), 4).await;
            if confirmed.is_none() && announce {
                // They cannot be reached over TCP; at least let them find us (UDP answer).
                let answer = app.me().announcement_json(false).to_string();
                discovery.send(answer.as_bytes()).await;
            }
            discovery.pending.lock().unwrap().remove(&fingerprint);
        });
    }

    async fn liveness_loop(&self, app: &Arc<App>) {
        loop {
            tokio::time::sleep(LIVENESS_INTERVAL).await;
            let stale: Vec<_> = app
                .registry
                .list()
                .into_iter()
                .filter(|p| p.kind == PeerKind::LocalSend && p.last_seen.elapsed() > STALE_AFTER)
                .collect();
            // A few at a time, however many devices there are.
            use futures_util::StreamExt;
            let results: Vec<_> = futures_util::stream::iter(stale.clone())
                .map(|peer| async move {
                    register_with(
                        app,
                        peer.ip,
                        peer.info.port,
                        peer.info.is_https(),
                        Some(peer.id()),
                        4,
                    )
                    .await
                })
                .buffered(16)
                .collect()
                .await;
            for (peer, result) in stale.iter().zip(results) {
                if result.is_none() {
                    app.registry.record_failure(peer.id(), MAX_FAILURES);
                }
            }
        }
    }

    /// Probes every host on the local networks (for networks without multicast).
    pub async fn scan(&self, app: &Arc<App>) -> usize {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return 0;
        }
        app.bus.emit("scan", json!({ "running": true }));
        let started = Instant::now();
        let semaphore = Arc::new(Semaphore::new(64));
        let mut ports = vec![crate::config::DEFAULT_PORT];
        let own_port = app.me().port;
        if own_port != crate::config::DEFAULT_PORT {
            ports.push(own_port);
        }
        let mut tasks = Vec::new();
        for ip in net::scan_targets() {
            for &port in &ports {
                let (app, semaphore) = (app.clone(), semaphore.clone());
                tasks.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await.ok()?;
                    for https in [true, false] {
                        if let Some(info) =
                            register_with(&app, IpAddr::V4(ip), port, https, None, 2).await
                        {
                            return Some(info);
                        }
                    }
                    None
                }));
            }
        }
        let found = futures_util::future::join_all(tasks)
            .await
            .into_iter()
            .filter(|r| matches!(r, Ok(Some(_))))
            .count();
        tracing::info!(
            "Network scan found {found} device(s) in {:.1}s",
            started.elapsed().as_secs_f32()
        );
        self.scanning.store(false, Ordering::SeqCst);
        app.bus
            .emit("scan", json!({ "running": false, "found": found }));
        found
    }
}

/// POST /register to a device; on success it is added to the registry.
pub async fn register_with(
    app: &App,
    ip: IpAddr,
    port: u16,
    https: bool,
    expected_fingerprint: Option<&str>,
    timeout_secs: u64,
) -> Option<DeviceInfo> {
    let target = Target {
        ip,
        port,
        https,
        fingerprint: expected_fingerprint.map(str::to_string),
    };
    let me = app.me();
    let options = Options {
        connect_timeout: Duration::from_secs(timeout_secs),
        response_timeout: Some(Duration::from_secs(timeout_secs)),
    };
    let response = match client::request(
        &app.identity,
        &target,
        Method::POST,
        &format!("{API}/register"),
        Body::json(&me.register_json()),
        options,
    )
    .await
    {
        Ok(response) if response.status == 200 => response,
        Ok(response) => {
            tracing::debug!("register with {ip}:{port}: HTTP {}", response.status);
            return None;
        }
        Err(err) => {
            tracing::debug!("register with {ip}:{port} failed: {err}");
            return None;
        }
    };
    let mut value: serde_json::Value = response.json().ok()?;
    // The response has no port/protocol: we know them from the request.
    value["port"] = json!(port);
    value["protocol"] = json!(if https { "https" } else { "http" });
    let mut info: DeviceInfo = serde_json::from_value(value).ok()?;
    info = info.sanitized().ok()?;
    // Over HTTPS the certificate is the identity, not what the body claims.
    let verified = response.peer_fingerprint.is_some();
    if let Some(fp) = response.peer_fingerprint {
        info.fingerprint = fp;
    }
    if !is_localsend_id(&info.fingerprint) || info.fingerprint.eq_ignore_ascii_case(&me.fingerprint)
    {
        return None;
    }
    app.registry
        .upsert(info.clone(), ip, PeerKind::LocalSend, verified);
    Some(info)
}
