//! Devices we know about (LocalSend peers and paired browsers).

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use crate::client::Target;
use crate::events::EventBus;
use crate::protocol::DeviceInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerKind {
    /// Speaks the LocalSend protocol (AirBridge for Android, LocalSend, …).
    LocalSend,
    /// A phone browser connected to the web companion.
    Web,
    /// An Android phone's built-in Quick Share, ready to receive.
    QuickShare,
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub info: DeviceInfo,
    pub ip: IpAddr,
    pub kind: PeerKind,
    pub last_seen: Instant,
    pub failures: u32,
    /// Its id was proven by a TLS certificate (or, for a browser, the pairing
    /// key), not just claimed in a packet.
    pub verified: bool,
}

impl Peer {
    pub fn id(&self) -> &str {
        &self.info.fingerprint
    }

    pub fn target(&self) -> Target {
        Target {
            ip: self.ip,
            port: self.info.port,
            https: self.info.is_https(),
            fingerprint: self.info.is_https().then(|| self.info.fingerprint.clone()),
        }
    }

    pub fn to_json(&self, trusted: bool) -> Value {
        json!({
            "id": self.info.fingerprint,
            "alias": self.info.alias,
            "deviceModel": self.info.device_model,
            "deviceType": self.info.device_type,
            "ip": self.ip.to_string(),
            "port": self.info.port,
            "protocol": self.info.protocol,
            "kind": self.kind,
            "trusted": trusted,
        })
    }
}

/// Most devices we keep at once; a flood of fake announcements evicts the
/// oldest untrusted ones instead of growing without bound.
const MAX_PEERS: usize = 256;

pub struct Registry {
    peers: Mutex<HashMap<String, Peer>>,
    trusted: RwLock<HashSet<String>>,
    bus: EventBus,
}

impl Registry {
    pub fn new(bus: EventBus, trusted: &[String]) -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
            trusted: RwLock::new(trusted.iter().cloned().collect()),
            bus,
        }
    }

    pub fn is_trusted(&self, id: &str) -> bool {
        self.trusted.read().unwrap().contains(id)
    }

    pub fn set_trusted(&self, id: &str, trusted: bool) {
        {
            let mut set = self.trusted.write().unwrap();
            if trusted {
                set.insert(id.to_string());
            } else {
                set.remove(id);
            }
        }
        if let Some(peer) = self.get(id) {
            self.bus.emit("peer", peer.to_json(trusted));
        }
    }

    pub fn peer_json(&self, peer: &Peer) -> Value {
        peer.to_json(self.is_trusted(peer.id()))
    }

    /// Adds or refreshes a device. `verified` says whether its id was proven
    /// (a TLS certificate, the browser pairing key) or only claimed.
    pub fn upsert(&self, info: DeviceInfo, ip: IpAddr, kind: PeerKind, verified: bool) -> Peer {
        let mut peers = self.peers.lock().unwrap();
        let id = info.fingerprint.clone();
        let unchanged = |peers: &HashMap<String, Peer>, info: DeviceInfo| {
            peers.get(&info.fingerprint).cloned().unwrap_or(Peer {
                info,
                ip,
                kind,
                last_seen: Instant::now(),
                failures: 0,
                verified,
            })
        };
        // A packet claiming a known device's id may not take it over: no
        // unverified update of a verified entry, no switch from HTTPS to plain
        // HTTP (which would drop the certificate pin), and your own devices
        // are only ever reached over HTTPS.
        if !verified {
            let downgrade = !info.is_https()
                && (kind == PeerKind::LocalSend && self.is_trusted(&id)
                    || peers.get(&id).is_some_and(|p| p.info.is_https()));
            if downgrade || peers.get(&id).is_some_and(|p| p.verified) {
                tracing::debug!("ignoring an unverified update for {id} from {ip}");
                return unchanged(&peers, info);
            }
        }
        // A computer running AirBridge also answers as a Quick Share device.
        // Show it once, like AirDrop does, through the richer LocalSend side.
        let speaks_localsend = |peers: &HashMap<String, Peer>| {
            peers
                .values()
                .any(|p| p.ip == ip && p.kind == PeerKind::LocalSend)
        };
        if kind == PeerKind::QuickShare && speaks_localsend(&peers) {
            return unchanged(&peers, info);
        }
        let mut merged = Vec::new();
        if kind == PeerKind::LocalSend {
            peers.retain(|other_id, p| {
                let duplicate = p.ip == ip && p.kind == PeerKind::QuickShare;
                if duplicate {
                    merged.push(other_id.clone());
                }
                !duplicate
            });
        }
        let (peer, changed) = match peers.get_mut(&id) {
            Some(existing) => {
                let changed = existing.info != info || existing.ip != ip || existing.kind != kind;
                existing.info = info;
                existing.ip = ip;
                existing.kind = kind;
                existing.last_seen = Instant::now();
                existing.failures = 0;
                existing.verified |= verified;
                (existing.clone(), changed)
            }
            None => {
                if peers.len() >= MAX_PEERS {
                    let trusted = self.trusted.read().unwrap();
                    let oldest = peers
                        .iter()
                        .filter(|(id, _)| !trusted.contains(*id))
                        .min_by_key(|(_, p)| p.last_seen)
                        .map(|(id, _)| id.clone());
                    match oldest {
                        Some(old) => {
                            peers.remove(&old);
                            merged.push(old);
                        }
                        None => return unchanged(&peers, info),
                    }
                }
                tracing::info!("Found {} ({ip}, {kind:?})", info.alias);
                let peer = Peer {
                    info,
                    ip,
                    kind,
                    last_seen: Instant::now(),
                    failures: 0,
                    verified,
                };
                peers.insert(id, peer.clone());
                (peer, true)
            }
        };
        drop(peers);
        for id in merged {
            self.bus.emit("peer-removed", json!({ "id": id }));
        }
        if changed {
            self.bus.emit("peer", self.peer_json(&peer));
        }
        peer
    }

    pub fn remove(&self, id: &str) {
        let removed = self.peers.lock().unwrap().remove(id);
        if let Some(peer) = removed {
            tracing::info!("Lost {}", peer.info.alias);
            self.bus.emit("peer-removed", json!({ "id": id }));
        }
    }

    /// Records a failed liveness check; returns true if the peer was dropped.
    pub fn record_failure(&self, id: &str, max_failures: u32) -> bool {
        let dropped = {
            let mut peers = self.peers.lock().unwrap();
            match peers.get_mut(id) {
                Some(peer) => {
                    peer.failures += 1;
                    peer.failures >= max_failures
                }
                None => false,
            }
        };
        if dropped {
            self.remove(id);
        }
        dropped
    }

    pub fn get(&self, id: &str) -> Option<Peer> {
        self.peers.lock().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Peer> {
        let mut list: Vec<Peer> = self.peers.lock().unwrap().values().cloned().collect();
        list.sort_by_key(|p| p.info.alias.to_lowercase());
        list
    }

    /// Finds a peer by id, exact alias, IP, or unique alias substring.
    pub fn find(&self, query: &str) -> Option<Peer> {
        let q = query.to_lowercase();
        let list = self.list();
        if let Some(p) = list.iter().find(|p| {
            p.id().to_lowercase() == q || p.info.alias.to_lowercase() == q || p.ip.to_string() == q
        }) {
            return Some(p.clone());
        }
        let matches: Vec<&Peer> = list
            .iter()
            .filter(|p| p.info.alias.to_lowercase().contains(&q))
            .collect();
        (matches.len() == 1).then(|| matches[0].clone())
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.list().iter().map(|p| self.peer_json(p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(fingerprint: &str) -> DeviceInfo {
        serde_json::from_value(json!({ "alias": "Studio PC", "fingerprint": fingerprint })).unwrap()
    }

    #[test]
    fn a_computer_speaking_both_protocols_shows_up_once() {
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        let other: IpAddr = "192.168.1.21".parse().unwrap();

        // Quick Share first, then LocalSend from the same address: merged.
        let registry = Registry::new(EventBus::default(), &[]);
        registry.upsert(device("qs"), ip, PeerKind::QuickShare, false);
        registry.upsert(device("qs-other"), other, PeerKind::QuickShare, false);
        registry.upsert(device("ls"), ip, PeerKind::LocalSend, true);
        let ids: Vec<String> = registry.list().iter().map(|p| p.id().to_string()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"ls".to_string()) && ids.contains(&"qs-other".to_string()));

        // And Quick Share announcing after LocalSend is not added.
        registry.upsert(device("qs"), ip, PeerKind::QuickShare, false);
        assert!(registry.get("qs").is_none());
    }

    fn localsend(fingerprint: &str, protocol: &str, port: u16) -> DeviceInfo {
        serde_json::from_value(json!({ "alias": "Phone", "fingerprint": fingerprint, "protocol": protocol, "port": port }))
            .unwrap()
    }

    #[test]
    fn a_claimed_id_cannot_take_over_a_device() {
        let phone: IpAddr = "192.168.1.30".parse().unwrap();
        let attacker: IpAddr = "192.168.1.66".parse().unwrap();
        let registry = Registry::new(EventBus::default(), &["MINE".repeat(16)]);
        let mine = "MINE".repeat(16);

        // A verified device keeps its address against unverified claims.
        registry.upsert(
            localsend(&mine, "https", 53317),
            phone,
            PeerKind::LocalSend,
            true,
        );
        registry.upsert(
            localsend(&mine, "https", 8080),
            attacker,
            PeerKind::LocalSend,
            false,
        );
        let peer = registry.get(&mine).unwrap();
        assert_eq!(
            (peer.ip, peer.info.port, peer.verified),
            (phone, 53317, true)
        );

        // Your own devices are never reached over plain HTTP, even unverified.
        let fresh = Registry::new(EventBus::default(), std::slice::from_ref(&mine));
        fresh.upsert(
            localsend(&mine, "http", 8080),
            attacker,
            PeerKind::LocalSend,
            false,
        );
        assert!(fresh.get(&mine).is_none());

        // And no device is downgraded from HTTPS to HTTP.
        let other = "OTHR".repeat(16);
        fresh.upsert(
            localsend(&other, "https", 53317),
            phone,
            PeerKind::LocalSend,
            false,
        );
        fresh.upsert(
            localsend(&other, "http", 8080),
            attacker,
            PeerKind::LocalSend,
            false,
        );
        assert!(fresh.get(&other).unwrap().info.is_https());
    }

    #[test]
    fn floods_do_not_grow_the_list_without_bound() {
        let registry = Registry::new(EventBus::default(), &[]);
        let ip: IpAddr = "192.168.1.66".parse().unwrap();
        for i in 0..(MAX_PEERS + 50) {
            registry.upsert(
                localsend(&format!("FAKE{i:08}"), "https", 53317),
                ip,
                PeerKind::LocalSend,
                false,
            );
        }
        assert_eq!(registry.list().len(), MAX_PEERS);
    }
}
