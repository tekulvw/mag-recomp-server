//! Request limits: per client address for the unauthenticated endpoints that create rows, and
//! per account for the signed-in APIs (keyed by account ID).

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Forget idle clients once this many are tracked.
const PRUNE_ABOVE: usize = 50_000;

/// Fixed-window counters per key: client addresses by default, or account IDs.
pub struct RateLimiter<K = IpAddr> {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<K, (Instant, u32)>>,
}

/// IPv6 clients usually hold a whole /64, so they share one bucket per /64.
fn client_key(ip: Option<IpAddr>) -> IpAddr {
    match ip {
        Some(IpAddr::V6(v6)) => {
            let mut octets = v6.octets();
            octets[8..].fill(0);
            IpAddr::from(octets)
        }
        Some(v4) => v4,
        // Unknown clients share one bucket.
        None => IpAddr::from([0, 0, 0, 0]),
    }
}

impl<K: Hash + Eq> RateLimiter<K> {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one request for `key`; false once the key has used up its window.
    pub fn allow_key(&self, key: K) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() > PRUNE_ABOVE {
            buckets.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let bucket = buckets.entry(key).or_insert((now, 0));
        if now.duration_since(bucket.0) >= self.window {
            *bucket = (now, 0);
        }
        if bucket.1 >= self.limit {
            return false;
        }
        bucket.1 += 1;
        true
    }
}

impl RateLimiter<IpAddr> {
    /// Counts one request; false once the client has used up its window.
    pub fn allow(&self, ip: Option<IpAddr>) -> bool {
        self.allow_key(client_key(ip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_client_and_prefix() {
        let limiter: RateLimiter = RateLimiter::new(2, Duration::from_secs(60));
        let a: IpAddr = "192.0.2.1".parse().unwrap();
        let b: IpAddr = "192.0.2.2".parse().unwrap();
        assert!(limiter.allow(Some(a)) && limiter.allow(Some(a)));
        assert!(!limiter.allow(Some(a)));
        assert!(limiter.allow(Some(b)));
        let v6a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let v6b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert!(limiter.allow(Some(v6a)) && limiter.allow(Some(v6b)));
        assert!(!limiter.allow(Some(v6a)), "one /64 shares a bucket");
    }

    #[test]
    fn windows_reset() {
        let limiter: RateLimiter = RateLimiter::new(1, Duration::from_millis(20));
        assert!(limiter.allow(None));
        assert!(!limiter.allow(None));
        std::thread::sleep(Duration::from_millis(25));
        assert!(limiter.allow(None));
    }

    #[test]
    fn limits_per_account() {
        let limiter: RateLimiter<i32> = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.allow_key(1) && limiter.allow_key(2));
        assert!(!limiter.allow_key(1));
    }
}
