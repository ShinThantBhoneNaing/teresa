//! Token-bucket rate limiting.
//!
//! One generic limiter covers all three uses — login attempts per IP, message
//! floods per connection, account creation per IP — differing only in their
//! bucket parameters. The implementation is a classic lazy token bucket: no
//! background task refills anything; a bucket's level is computed from the
//! time elapsed since it was last touched.

use std::hash::Hash;
use std::time::{Duration, Instant};

use dashmap::DashMap;

/// A shared, keyed token bucket.
///
/// `K` is whatever identifies a caller: an `IpAddr`, a `UserId`, a
/// `ConnectionId`. Cloning is cheap and shares the underlying map.
#[derive(Debug)]
pub struct RateLimiter<K: Eq + Hash> {
    buckets: DashMap<K, Bucket>,
    /// Tokens a full bucket holds; also the burst size.
    capacity: f64,
    /// Tokens restored per second.
    refill_per_sec: f64,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    /// Tokens available right now (as of `touched`).
    level: f64,
    /// When `level` was last recomputed.
    touched: Instant,
}

/// The outcome of asking for one token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    /// Allowed; the token was spent.
    Allow,
    /// Refused; retry after roughly this long.
    Deny {
        /// Time until one full token will have refilled.
        retry_after: Duration,
    },
}

impl Decision {
    /// Whether the request was allowed.
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// The retry hint in whole seconds, rounded up so a client that obeys it
    /// exactly is never refused twice.
    #[must_use]
    pub fn retry_after_secs(&self) -> u64 {
        match self {
            Self::Allow => 0,
            Self::Deny { retry_after } => retry_after.as_secs_f64().ceil() as u64,
        }
    }
}

impl<K: Eq + Hash> RateLimiter<K> {
    /// A limiter allowing `count` events per `window`, with bursts up to `count`.
    ///
    /// # Panics
    /// Panics if `count` is zero or `window` is zero; config validation
    /// guarantees neither reaches production.
    #[must_use]
    pub fn per_window(count: u32, window: Duration) -> Self {
        assert!(count > 0, "a zero-rate limiter would deny everything");
        assert!(!window.is_zero(), "a zero window is meaningless");
        Self {
            buckets: DashMap::new(),
            capacity: f64::from(count),
            refill_per_sec: f64::from(count) / window.as_secs_f64(),
        }
    }

    /// Spends one token for `key`, refusing if the bucket is empty.
    pub fn check(&self, key: K) -> Decision {
        self.check_at(key, Instant::now())
    }

    /// [`RateLimiter::check`] with an explicit clock, so tests need not sleep.
    fn check_at(&self, key: K, now: Instant) -> Decision {
        let mut entry = self.buckets.entry(key).or_insert(Bucket {
            level: self.capacity,
            touched: now,
        });

        // Refill for the time elapsed, capped at capacity.
        let elapsed = now.saturating_duration_since(entry.touched).as_secs_f64();
        entry.level = (entry.level + elapsed * self.refill_per_sec).min(self.capacity);
        entry.touched = now;

        if entry.level >= 1.0 {
            entry.level -= 1.0;
            Decision::Allow
        } else {
            let deficit = 1.0 - entry.level;
            Decision::Deny {
                retry_after: Duration::from_secs_f64(deficit / self.refill_per_sec),
            }
        }
    }

    /// Discards buckets that have been idle long enough to be full again.
    ///
    /// Called periodically by the server's housekeeping task; without it, every
    /// IP that ever connected would keep an entry forever. A full bucket and a
    /// missing bucket behave identically, so this changes no decisions.
    pub fn prune(&self) {
        let now = Instant::now();
        let refill_to_full = self.capacity / self.refill_per_sec;
        self.buckets.retain(|_, bucket| {
            now.saturating_duration_since(bucket.touched).as_secs_f64() < refill_to_full
        });
    }

    /// The number of live buckets, for the metrics endpoint.
    #[must_use]
    pub fn tracked_keys(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_burst_then_denies() {
        let limiter = RateLimiter::per_window(3, Duration::from_secs(60));
        let now = Instant::now();

        assert!(limiter.check_at("10.0.0.1", now).is_allowed());
        assert!(limiter.check_at("10.0.0.1", now).is_allowed());
        assert!(limiter.check_at("10.0.0.1", now).is_allowed());
        assert!(!limiter.check_at("10.0.0.1", now).is_allowed());
    }

    #[test]
    fn keys_do_not_interfere() {
        let limiter = RateLimiter::per_window(1, Duration::from_secs(60));
        let now = Instant::now();

        assert!(limiter.check_at("attacker", now).is_allowed());
        assert!(!limiter.check_at("attacker", now).is_allowed());
        assert!(limiter.check_at("bystander", now).is_allowed());
    }

    #[test]
    fn tokens_refill_with_time() {
        let limiter = RateLimiter::per_window(2, Duration::from_secs(10));
        let start = Instant::now();

        assert!(limiter.check_at("k", start).is_allowed());
        assert!(limiter.check_at("k", start).is_allowed());
        assert!(!limiter.check_at("k", start).is_allowed());

        // One token refills every 5 seconds at 2-per-10s.
        assert!(limiter
            .check_at("k", start + Duration::from_secs(6))
            .is_allowed());
        assert!(!limiter
            .check_at("k", start + Duration::from_secs(6))
            .is_allowed());
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let limiter = RateLimiter::per_window(2, Duration::from_secs(1));
        let start = Instant::now();

        // A long sleep must not bank more than the burst size.
        assert!(limiter.check_at("k", start).is_allowed());
        let later = start + Duration::from_secs(3600);
        assert!(limiter.check_at("k", later).is_allowed());
        assert!(limiter.check_at("k", later).is_allowed());
        assert!(!limiter.check_at("k", later).is_allowed());
    }

    #[test]
    fn deny_reports_an_honest_retry_hint() {
        let limiter = RateLimiter::per_window(1, Duration::from_secs(10));
        let start = Instant::now();

        assert!(limiter.check_at("k", start).is_allowed());
        let decision = limiter.check_at("k", start);
        // A full token takes 10s to refill; the hint rounds up.
        assert_eq!(decision.retry_after_secs(), 10);

        // And obeying the hint must succeed.
        let after = start + Duration::from_secs(decision.retry_after_secs());
        assert!(limiter.check_at("k", after).is_allowed());
    }

    #[test]
    fn prune_discards_only_full_buckets() {
        let limiter = RateLimiter::per_window(1, Duration::from_millis(1));
        assert!(limiter.check("stale").is_allowed());
        assert_eq!(limiter.tracked_keys(), 1);

        // 1-per-1ms refills to full almost immediately.
        std::thread::sleep(Duration::from_millis(20));
        limiter.prune();
        assert_eq!(limiter.tracked_keys(), 0);
    }

    #[test]
    fn a_pruned_key_starts_fresh() {
        let limiter = RateLimiter::per_window(1, Duration::from_millis(1));
        assert!(limiter.check("k").is_allowed());
        std::thread::sleep(Duration::from_millis(20));
        limiter.prune();
        assert!(limiter.check("k").is_allowed());
    }
}
