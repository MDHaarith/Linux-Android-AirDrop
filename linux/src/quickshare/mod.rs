//! Google Quick Share (formerly Nearby Share) interoperability, so a stock
//! Android phone can share with this computer from its built-in share sheet,
//! and this computer can send to phones that are ready to receive.
//!
//! Only the Wi-Fi LAN medium is implemented: devices find each other with
//! mDNS and talk over TCP, end-to-end encrypted with UKEY2 + SecureMessage.

mod ble;
mod crypto;
mod inbound;
mod link;
mod outbound;
mod proto;
mod wire;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::peers::PeerKind;
use crate::protocol::DeviceInfo;

pub use outbound::{send_files, send_text};
use wire::{DeviceKind, EndpointInfo};

/// How long phones are asked to stay visible after each wake-up.
const WAKE_WINDOW: Duration = Duration::from_secs(60);

pub struct Service {
    /// Identifies this computer to phones for as long as the service runs.
    pub endpoint_id: [u8; 4],
    pub port: u16,
    daemon: ServiceDaemon,
    fullname: Mutex<Option<String>>,
    stop: CancellationToken,
    wake: watch::Sender<Option<Instant>>,
}

impl Service {
    pub async fn start(app: &Arc<App>) -> Result<Arc<Self>> {
        // A fixed port is easier to allow through a firewall.
        let listener =
            match TcpListener::bind((Ipv4Addr::UNSPECIFIED, crate::config::QUICK_SHARE_PORT)).await
            {
                Ok(listener) => listener,
                Err(_) => TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).await?,
            };
        let port = listener.local_addr()?.port();
        let daemon = ServiceDaemon::new().map_err(|e| anyhow!("mDNS unavailable: {e}"))?;
        // We listen on IPv4 only, so only advertise IPv4 addresses, and
        // never 127.0.0.1 (a phone would try to connect to itself).
        let _ = daemon.disable_interface(IfKind::IPv6);
        let _ = daemon.disable_interface(IfKind::LoopbackV4);
        let service = Arc::new(Self {
            endpoint_id: wire::random_endpoint_id(),
            port,
            daemon,
            fullname: Mutex::new(None),
            stop: CancellationToken::new(),
            wake: watch::Sender::new(None),
        });
        service.advertise(&app.settings().alias)?;
        tokio::spawn(accept_loop(app.clone(), listener, service.stop.clone()));
        match service.daemon.browse(wire::SERVICE_TYPE) {
            Ok(events) => {
                tokio::spawn(browse_loop(app.clone(), service.clone(), events));
            }
            Err(err) => tracing::warn!("Cannot look for Quick Share devices: {err}"),
        }
        tokio::spawn(beacon_loop(service.clone(), service.wake.subscribe()));
        tracing::info!("Quick Share: visible to Android phones on port {port}");
        Ok(service)
    }

    /// Publishes (or updates) the mDNS record phones look for.
    pub fn advertise(&self, alias: &str) -> Result<()> {
        let info = EndpointInfo {
            kind: DeviceKind::Laptop,
            name: Some(alias.to_string()),
        };
        let host = format!(
            "airbridge-{}.local.",
            String::from_utf8_lossy(&self.endpoint_id).to_lowercase()
        );
        let record = ServiceInfo::new(
            wire::SERVICE_TYPE,
            &wire::service_name(&self.endpoint_id),
            &host,
            "",
            self.port,
            &[("n", info.to_base64())][..],
        )?
        .enable_addr_auto();
        let fullname = record.get_fullname().to_string();
        self.daemon.register(record)?;
        *self.fullname.lock().unwrap() = Some(fullname);
        Ok(())
    }

    /// Asks nearby phones (over Bluetooth) to show up as send targets for
    /// the next minute. Called when the user shows intent to share.
    pub fn wake_phones(&self) {
        let until = Some(Instant::now() + WAKE_WINDOW);
        self.wake.send_if_modified(|deadline| {
            let later = until > *deadline;
            if later {
                *deadline = until;
            }
            later
        });
    }

    pub fn stop(&self) {
        self.stop.cancel();
        if let Some(fullname) = self.fullname.lock().unwrap().take() {
            if let Ok(done) = self.daemon.unregister(&fullname) {
                // Sends the "goodbye" so phones drop us right away.
                let _ = done.recv_timeout(std::time::Duration::from_secs(1));
            }
        }
        let _ = self.daemon.shutdown();
    }
}

async fn accept_loop(app: Arc<App>, listener: TcpListener, stop: CancellationToken) {
    // A phone shares over one connection; a handful covers several phones.
    let limits = crate::limits::ConnLimit::new(8, 2);
    loop {
        let (stream, addr) = tokio::select! {
            _ = stop.cancelled() => return,
            _ = app.shutdown.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(err) => {
                    tracing::warn!("Quick Share: accept failed: {err}");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
            },
        };
        let Some(permit) = limits.try_acquire(crate::net::canonical(addr.ip())) else {
            tracing::debug!("Quick Share: too many connections from {addr}");
            continue;
        };
        let app = app.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(err) = inbound::handle(app, stream).await {
                tracing::info!("Quick Share connection from {addr} ended: {err:#}");
            }
        });
    }
}

/// Broadcasts the Bluetooth "nearby sharing" advertisement until the wake
/// deadline passes. Without Bluetooth, phones show up only while their
/// Quick Share "receive" screen is open.
async fn beacon_loop(service: Arc<Service>, mut wake: watch::Receiver<Option<Instant>>) {
    let mut beacon: Option<ble::Beacon> = None;
    let mut retry_after: Option<Instant> = None;
    loop {
        let deadline = *wake.borrow_and_update();
        let now = Instant::now();
        let active = deadline.is_some_and(|until| until > now);
        if active && beacon.is_none() && retry_after.is_none_or(|t| t <= now) {
            let created = async { ble::Beacon::new(zbus::Connection::system().await?).await };
            match created.await {
                Ok(created) => beacon = Some(created),
                Err(err) => {
                    tracing::info!(
                        "Quick Share: cannot wake nearby phones over Bluetooth: {err:#}"
                    );
                    retry_after = Some(now + Duration::from_secs(300));
                }
            }
        }
        if let Some(current) = beacon.as_mut() {
            let result = if active {
                current.start().await
            } else {
                current.stop().await;
                Ok(())
            };
            if let Err(err) = result {
                tracing::info!("Quick Share: Bluetooth advertisement failed: {err:#}");
                beacon = None;
                retry_after = Some(now + Duration::from_secs(60));
            }
        }
        let sleep = async {
            match deadline {
                Some(until) if active => tokio::time::sleep_until(until).await,
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = service.stop.cancelled() => break,
            _ = sleep => {}
            changed = wake.changed() => if changed.is_err() { break },
        }
    }
    if let Some(mut beacon) = beacon {
        beacon.stop().await;
    }
}

fn peer_id(endpoint_id: &[u8; 4]) -> String {
    format!("quickshare:{}", String::from_utf8_lossy(endpoint_id))
}

/// Phones advertise only while their "receive" screen is open (or after a
/// nearby device asked them to); those become send targets.
async fn browse_loop(
    app: Arc<App>,
    service: Arc<Service>,
    events: mdns_sd::Receiver<ServiceEvent>,
) {
    let mut known: HashMap<String, String> = HashMap::new();
    loop {
        let event = tokio::select! {
            _ = service.stop.cancelled() => break,
            event = events.recv_async() => match event {
                Ok(event) => event,
                Err(_) => break,
            },
        };
        match event {
            ServiceEvent::ServiceResolved(resolved) => {
                let instance = resolved.fullname.split('.').next().unwrap_or_default();
                let Some(endpoint_id) = wire::parse_service_name(instance) else {
                    continue;
                };
                if endpoint_id == service.endpoint_id {
                    continue;
                }
                let Some(info) = resolved
                    .get_property_val_str("n")
                    .and_then(EndpointInfo::from_base64)
                else {
                    continue;
                };
                // Hidden devices (contacts-only visibility) need Google accounts.
                let Some(name) = info.name.clone().filter(|n| !n.trim().is_empty()) else {
                    continue;
                };
                // Only a phone on our own network, on a normal port: a record
                // pointing at this computer or some other service is ignored.
                let mut addresses: Vec<Ipv4Addr> = resolved
                    .get_addresses_v4()
                    .into_iter()
                    .filter(|ip| {
                        !ip.is_loopback()
                            && !ip.is_unspecified()
                            && !ip.is_multicast()
                            && !ip.is_broadcast()
                            && crate::net::is_local_network(IpAddr::V4(*ip))
                    })
                    .collect();
                addresses.sort_by_key(|ip| ip.is_link_local());
                let Some(&ip) = addresses.first() else {
                    continue;
                };
                if resolved.port < 1024 {
                    continue;
                }
                let id = peer_id(&endpoint_id);
                // A record reusing another phone's id does not take it over.
                if app
                    .registry
                    .get(&id)
                    .is_some_and(|known| known.ip != IpAddr::V4(ip))
                {
                    continue;
                }
                let device = DeviceInfo {
                    alias: name.chars().filter(|c| !c.is_control()).take(64).collect(),
                    version: "quickshare".into(),
                    device_model: Some("Quick Share".into()),
                    device_type: Some(info.kind.as_device_type().into()),
                    fingerprint: id.clone(),
                    port: resolved.port,
                    protocol: "quickshare".into(),
                    download: false,
                };
                app.registry
                    .upsert(device, IpAddr::V4(ip), PeerKind::QuickShare, false);
                known.insert(resolved.fullname.to_lowercase(), id);
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                if let Some(id) = known.remove(&fullname.to_lowercase()) {
                    app.registry.remove(&id);
                }
            }
            _ => {}
        }
    }
    for id in known.values() {
        app.registry.remove(id);
    }
}

#[cfg(test)]
mod tests;
