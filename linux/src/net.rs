//! Local network helpers.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Virtual interfaces that never lead to a phone on the same Wi-Fi.
const IGNORED_PREFIXES: &[&str] =
    &["docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "lxc", "cni", "flannel", "tailscale", "zt"];

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

impl Interface {
    pub fn is_wireless(&self) -> bool {
        self.name.starts_with("wl") || self.name.starts_with("wifi")
    }

    pub fn prefix_len(&self) -> u32 {
        u32::from(self.netmask).count_ones()
    }
}

/// IPv4 LAN interfaces that are up, wireless ones first.
pub fn interfaces() -> Vec<Interface> {
    let mut result: Vec<Interface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && !IGNORED_PREFIXES.iter().any(|p| i.name.starts_with(p)))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(Interface { name: i.name, ip: v4.ip, netmask: v4.netmask }),
            _ => None,
        })
        .filter(|i| !i.ip.is_link_local())
        .collect();
    result.sort_by_key(|i| (!i.is_wireless(), i.name.clone()));
    result.dedup_by(|a, b| a.ip == b.ip);
    result
}

/// The address the kernel would use for LAN traffic (shown in the UI and QR code).
pub fn primary_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // connect() on UDP only selects a route; nothing is sent.
    if socket.connect("10.255.255.255:1").is_ok() {
        if let Ok(addr) = socket.local_addr() {
            if let IpAddr::V4(ip) = addr.ip() {
                if !ip.is_loopback() && !ip.is_unspecified() {
                    return Some(ip);
                }
            }
        }
    }
    interfaces().first().map(|i| i.ip)
}

/// Every host on the local networks, for the HTTP fallback scan.
pub fn scan_targets() -> Vec<Ipv4Addr> {
    let own: Vec<Ipv4Addr> = interfaces().iter().map(|i| i.ip).collect();
    let mut hosts = Vec::new();
    for iface in interfaces() {
        // Networks bigger than a /22 are scanned as the /24 around us.
        let prefix = if iface.prefix_len() < 22 { 24 } else { iface.prefix_len() };
        let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
        let network = u32::from(iface.ip) & mask;
        let broadcast = network | !mask;
        for host in (network + 1)..broadcast {
            let ip = Ipv4Addr::from(host);
            if !own.contains(&ip) && !hosts.contains(&ip) {
                hosts.push(ip);
            }
        }
    }
    hosts
}

/// IPv4-mapped IPv6 addresses (from dual-stack sockets) shown as plain IPv4.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}
