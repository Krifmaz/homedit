//! Failed-login throttling.
//!
//! Failures are counted per key in a fixed window. Two keys are checked for
//! each attempt: the client IP and the (lowercased) username, so neither
//! spraying one password over many accounts from one host nor guessing one
//! account from many hosts gets far. A locked key is rejected without
//! touching the password hash.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::RateLimitConfig;

#[derive(Debug, Clone, Copy)]
struct Entry {
    window_start: Instant,
    failures: u32,
    locked_until: Option<Instant>,
}

/// In-memory login limiter. Cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct LoginLimiter {
    cfg: RateLimitConfig,
    entries: Mutex<HashMap<String, Entry>>,
}

/// Upper bound on tracked keys; beyond this, stale entries are swept.
const MAX_KEYS: usize = 10_000;

impl LoginLimiter {
    /// Creates a limiter with the given settings.
    pub fn new(cfg: RateLimitConfig) -> Self {
        Self {
            cfg,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Keys for a login attempt.
    pub fn keys(ip: Option<std::net::IpAddr>, username: &str) -> [String; 2] {
        [
            format!(
                "ip:{}",
                ip.map(|i| i.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
            format!("user:{}", username.to_lowercase()),
        ]
    }

    /// If any key is locked, the number of seconds until it unlocks.
    pub fn check(&self, keys: &[String]) -> Option<u64> {
        self.check_at(keys, Instant::now())
    }

    fn check_at(&self, keys: &[String], now: Instant) -> Option<u64> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        keys.iter()
            .filter_map(|k| map.get(k)?.locked_until)
            .filter(|until| *until > now)
            .map(|until| (until - now).as_secs().max(1))
            .max()
    }

    /// Records a failed attempt for every key.
    pub fn record_failure(&self, keys: &[String]) {
        self.record_failure_at(keys, Instant::now());
    }

    fn record_failure_at(&self, keys: &[String], now: Instant) {
        let window = Duration::from_secs(self.cfg.window_secs);
        let lockout = Duration::from_secs(self.cfg.lockout_secs);
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS {
            map.retain(|_, e| {
                now.duration_since(e.window_start) < window
                    || e.locked_until.is_some_and(|u| u > now)
            });
        }
        for key in keys {
            let e = map.entry(key.clone()).or_insert(Entry {
                window_start: now,
                failures: 0,
                locked_until: None,
            });
            if now.duration_since(e.window_start) >= window {
                *e = Entry {
                    window_start: now,
                    failures: 0,
                    locked_until: None,
                };
            }
            e.failures += 1;
            if e.failures >= self.cfg.max_failures.max(1) {
                e.locked_until = Some(now + lockout);
                tracing::warn!(key = %key, failures = e.failures, "login locked after repeated failures");
            }
        }
    }

    /// Clears the username key after a successful login. The IP key is kept,
    /// so a host cannot reset its counter by logging into its own account.
    pub fn record_success(&self, username_key: &str) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(username_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> LoginLimiter {
        LoginLimiter::new(RateLimitConfig {
            max_failures: 3,
            window_secs: 60,
            lockout_secs: 120,
        })
    }

    #[test]
    fn locks_after_max_failures_and_unlocks_later() {
        let l = limiter();
        let keys = LoginLimiter::keys(Some("10.0.0.5".parse().unwrap()), "Alice");
        let t0 = Instant::now();
        for _ in 0..2 {
            l.record_failure_at(&keys, t0);
        }
        assert_eq!(l.check_at(&keys, t0), None);
        l.record_failure_at(&keys, t0);
        assert!(l.check_at(&keys, t0).unwrap() >= 119);
        assert_eq!(l.check_at(&keys, t0 + Duration::from_secs(121)), None);
    }

    #[test]
    fn username_is_case_insensitive_and_ip_survives_success() {
        let l = limiter();
        let a = LoginLimiter::keys(None, "Bob");
        let b = LoginLimiter::keys(None, "bob");
        assert_eq!(a[1], b[1]);
        let t0 = Instant::now();
        l.record_failure_at(&a, t0);
        l.record_failure_at(&a, t0);
        l.record_success(&a[1]);
        l.record_failure_at(&a, t0);
        // ip key reached 3, user key only 1
        assert!(l.check_at(&a[..1], t0).is_some());
        assert!(l.check_at(&a[1..], t0).is_none());
    }

    #[test]
    fn window_resets() {
        let l = limiter();
        let k = LoginLimiter::keys(None, "c");
        let t0 = Instant::now();
        l.record_failure_at(&k, t0);
        l.record_failure_at(&k, t0);
        l.record_failure_at(&k, t0 + Duration::from_secs(61));
        assert!(l.check_at(&k, t0 + Duration::from_secs(61)).is_none());
    }
}
