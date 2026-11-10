//! Local network helpers.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// Virtual interfaces that never lead to a phone on the same Wi-Fi.
const IGNORED_PREFIXES: &[&str] = &[
    "docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "lxc", "cni", "flannel",
];

#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

impl Interface {
    pub fn is_wireless(&self) -> bool {
        #[cfg(windows)]
        {
            let name = self.name.to_ascii_lowercase();
            name.starts_with("wl") || name.starts_with("wi-fi") || name.starts_with("wifi")
        }
        #[cfg(target_os = "linux")]
        {
            self.name.starts_with("wl") || self.name.starts_with("wifi")
        }
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
        .filter(|i| !i.name.starts_with("tailscale") && !i.name.starts_with("zt"))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(Interface {
                name: i.name,
                ip: v4.ip,
                netmask: v4.netmask,
            }),
            _ => None,
        })
        .filter(|i| !i.ip.is_link_local())
        .filter(|i| {
            // Windows adapter names may be GUIDs rather than friendly names.
            !cfg!(windows) || !is_cgnat(i.ip)
        })
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

fn is_cgnat(ip: Ipv4Addr) -> bool {
    (u32::from(ip) & 0xffc0_0000) == u32::from(Ipv4Addr::new(100, 64, 0, 0))
}

/// Addresses safe to remember and probe for paired devices (RFC 1918 and CGNAT).
pub fn is_pair_address(ip: Ipv4Addr) -> bool {
    ip.is_private() || is_cgnat(ip)
}

/// Pairing also advertises VPN interfaces, without scanning their subnets.
pub fn pair_addresses() -> Vec<Ipv4Addr> {
    let addresses: Vec<_> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !IGNORED_PREFIXES.iter().any(|p| i.name.starts_with(p)))
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4.ip),
            _ => None,
        })
        .collect();
    let lan = interfaces();
    let primary = primary_ipv4()
        .filter(|ip| lan.iter().any(|iface| iface.ip == *ip))
        .or_else(|| lan.first().map(|iface| iface.ip));
    ordered_pair_addresses(primary, addresses)
}

fn ordered_pair_addresses(
    primary: Option<Ipv4Addr>,
    addresses: impl IntoIterator<Item = Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let mut result = Vec::new();
    for ip in primary.into_iter().chain(addresses) {
        if is_pair_address(ip) && !result.contains(&ip) {
            result.push(ip);
            if result.len() == 4 {
                break;
            }
        }
    }
    result
}

/// Every host on the local networks, for the HTTP fallback scan.
pub fn scan_targets() -> Vec<Ipv4Addr> {
    let own: Vec<Ipv4Addr> = interfaces().iter().map(|i| i.ip).collect();
    let mut hosts = Vec::new();
    for iface in interfaces() {
        // Networks bigger than a /22 are scanned as the /24 around us.
        let prefix = if iface.prefix_len() < 22 {
            24
        } else {
            iface.prefix_len()
        };
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
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
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    }
}

/// Whether `ip` belongs to a network this computer is on (or is this
/// computer). Discovery packets from anywhere else are spoofed or routed in
/// from outside and are ignored.
pub fn is_local_network(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || interfaces().iter().any(|iface| {
                    let mask = u32::from(iface.netmask);
                    u32::from(iface.ip) & mask == u32::from(v4) & mask
                })
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_only_uses_private_and_cgnat_ipv4() {
        for ip in [
            "10.0.0.0",
            "10.255.255.255",
            "172.16.0.0",
            "172.31.255.255",
            "192.168.0.0",
            "192.168.255.255",
            "100.64.0.0",
            "100.127.255.255",
        ] {
            assert!(is_pair_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "0.0.0.0",
            "127.0.0.1",
            "169.254.1.1",
            "172.15.255.255",
            "172.32.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "8.8.8.8",
            "224.0.0.1",
        ] {
            assert!(!is_pair_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn pairing_addresses_prefer_primary_deduplicate_and_limit() {
        let ips = [
            "100.64.1.2",
            "192.168.1.2",
            "10.0.0.2",
            "172.16.1.2",
            "10.1.1.2",
        ]
        .map(|ip| ip.parse().unwrap());
        assert_eq!(
            ordered_pair_addresses(Some(ips[1]), ips),
            vec![ips[1], ips[0], ips[2], ips[3]]
        );
        assert_eq!(ordered_pair_addresses(None, [ips[0]]), vec![ips[0]]);
        assert_eq!(
            ordered_pair_addresses(Some(Ipv4Addr::LOCALHOST), [ips[0]]),
            vec![ips[0]]
        );
    }
}
