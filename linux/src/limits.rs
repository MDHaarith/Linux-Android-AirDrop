//! Limits against floods from the network: how often an address may do
//! something, and how many connections it may hold open.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// IPv6 senders can pick any address in their /64, so count them together.
fn bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        other => other,
    }
}

/// At most `max` events per address within `window`, plus an optional cap
/// for everyone together (so rotating addresses does not help).
pub struct RateLimit {
    max: usize,
    max_total: usize,
    window: Duration,
    hits: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    total: Mutex<VecDeque<Instant>>,
}

impl RateLimit {
    pub fn new(max: usize, max_total: usize, window: Duration) -> Self {
        Self {
            max,
            max_total,
            window,
            hits: Mutex::new(HashMap::new()),
            total: Mutex::new(VecDeque::new()),
        }
    }

    fn prune(window: Duration, list: &mut VecDeque<Instant>) {
        while list.front().is_some_and(|t| t.elapsed() >= window) {
            list.pop_front();
        }
    }

    /// Whether `ip` is still under the limit (without counting an event).
    pub fn allows(&self, ip: IpAddr) -> bool {
        let mut total = self.total.lock().unwrap();
        Self::prune(self.window, &mut total);
        if total.len() >= self.max_total {
            return false;
        }
        let mut hits = self.hits.lock().unwrap();
        let list = hits.entry(bucket(ip)).or_default();
        Self::prune(self.window, list);
        list.len() < self.max
    }

    /// Counts an event from `ip`.
    pub fn record(&self, ip: IpAddr) {
        let mut total = self.total.lock().unwrap();
        total.push_back(Instant::now());
        if total.len() > self.max_total * 2 {
            total.pop_front();
        }
        let mut hits = self.hits.lock().unwrap();
        hits.entry(bucket(ip))
            .or_default()
            .push_back(Instant::now());
        if hits.len() > 4096 {
            let window = self.window;
            hits.retain(|_, list| {
                Self::prune(window, list);
                !list.is_empty()
            });
        }
    }

    /// Counts an event and says whether it was allowed.
    pub fn hit(&self, ip: IpAddr) -> bool {
        let allowed = self.allows(ip);
        if allowed {
            self.record(ip);
        }
        allowed
    }
}

/// Caps open connections, in total and per address.
pub struct ConnLimit {
    total: Arc<Semaphore>,
    per_ip: usize,
    open: Mutex<HashMap<IpAddr, usize>>,
}

pub struct ConnPermit {
    limit: Arc<ConnLimit>,
    ip: IpAddr,
    _permit: OwnedSemaphorePermit,
}

impl ConnLimit {
    pub fn new(total: usize, per_ip: usize) -> Arc<Self> {
        Arc::new(Self {
            total: Arc::new(Semaphore::new(total)),
            per_ip,
            open: Mutex::new(HashMap::new()),
        })
    }

    /// A permit for one more connection from `ip`, or None when it (or
    /// everyone together) already has too many open.
    pub fn try_acquire(self: &Arc<Self>, ip: IpAddr) -> Option<ConnPermit> {
        let ip = bucket(ip);
        let mut open = self.open.lock().unwrap();
        let count = open.entry(ip).or_default();
        if *count >= self.per_ip {
            return None;
        }
        let permit = self.total.clone().try_acquire_owned().ok()?;
        *count += 1;
        Some(ConnPermit {
            limit: self.clone(),
            ip,
            _permit: permit,
        })
    }
}

impl Drop for ConnPermit {
    fn drop(&mut self) {
        let mut open = self.limit.open.lock().unwrap();
        if let Some(count) = open.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_address_and_in_total() {
        let limit = RateLimit::new(2, 3, Duration::from_secs(60));
        let a: IpAddr = "192.168.1.5".parse().unwrap();
        let b: IpAddr = "192.168.1.6".parse().unwrap();
        assert!(limit.hit(a) && limit.hit(a));
        assert!(!limit.hit(a), "per address");
        assert!(limit.hit(b));
        assert!(!limit.hit(b), "in total");
        // Addresses in one IPv6 /64 count as one.
        let limit = RateLimit::new(1, 100, Duration::from_secs(60));
        assert!(limit.hit("fd00::1".parse().unwrap()));
        assert!(!limit.hit("fd00::2".parse().unwrap()));
    }

    #[test]
    fn connections_are_capped() {
        let limit = ConnLimit::new(3, 2);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let first = limit.try_acquire(a).unwrap();
        let _second = limit.try_acquire(a).unwrap();
        assert!(limit.try_acquire(a).is_none(), "per address");
        let _other = limit.try_acquire("10.0.0.2".parse().unwrap()).unwrap();
        assert!(
            limit.try_acquire("10.0.0.3".parse().unwrap()).is_none(),
            "in total"
        );
        drop(first);
        assert!(limit.try_acquire(a).is_some());
    }
}
