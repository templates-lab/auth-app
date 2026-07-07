//! An in-memory, fixed-window rate limiter for sensitive endpoints (bead
//! authapp-5af1bb).
//!
//! Complements Traefik's edge-wide limit (`infra/traefik/dynamic/middlewares.yml`,
//! `api-ratelimit`) rather than replacing it: Traefik counts every request
//! across the whole API regardless of route or body, which is the right tool
//! for blunt, global protection, but it cannot see the request body — so it
//! cannot limit "attempts against this one account" the way this limiter
//! does. Keyed generically (a caller-chosen string), so the same type serves
//! login-by-IP, login-by-account, and — once that endpoint exists —
//! payment-webhook limiting, without duplicating the counter logic per route.
//!
//! In-memory and per-process by design: a defense-in-depth layer behind
//! Traefik's shared, cross-replica limit, not a replacement for it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// How many requests a key may make per window, and how long that window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Requests allowed per key within one window.
    pub max_requests: u32,
    /// The window's duration; it rolls over (resetting the count) once this
    /// much time has passed since it started.
    pub window: Duration,
}

/// A fixed-window rate limiter keyed by an arbitrary string.
///
/// "Fixed window" (rather than sliding or token-bucket) is a deliberate
/// simplicity choice: it can very slightly over-admit right at a window
/// boundary (up to `2 * max_requests` across two adjacent windows in the
/// worst case), which is an acceptable trade for a defense-in-depth layer
/// behind Traefik's own limit, in exchange for a trivially simple, easily
/// tested implementation.
#[derive(Debug)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
    config: RateLimitConfig,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    count: u32,
    started_at: SystemTime,
}

impl RateLimiter {
    /// Build a limiter with the given per-key config.
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Record one request for `key` at instant `now`.
    ///
    /// `Ok(())` if `key` is still within its window's limit — including the
    /// request that just consumed the last slot. `Err` once a key's count
    /// would exceed the configured maximum, carrying how long the caller
    /// should wait before its window rolls over.
    pub fn check(&self, key: &str, now: SystemTime) -> Result<(), RateLimitExceeded> {
        let mut windows = self.windows.lock().unwrap();
        let window = windows.entry(key.to_string()).or_insert(Window {
            count: 0,
            started_at: now,
        });

        let elapsed = now.duration_since(window.started_at).unwrap_or_default();
        if elapsed >= self.config.window {
            window.count = 0;
            window.started_at = now;
        }

        window.count += 1;
        if window.count > self.config.max_requests {
            let elapsed_in_current_window =
                now.duration_since(window.started_at).unwrap_or_default();
            Err(RateLimitExceeded {
                retry_after: self.config.window.saturating_sub(elapsed_in_current_window),
            })
        } else {
            Ok(())
        }
    }
}

/// A key exceeded its rate limit.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitExceeded {
    /// How long until this key's window rolls over and it may try again.
    pub retry_after: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(max_requests: u32, window_secs: u64) -> RateLimitConfig {
        RateLimitConfig {
            max_requests,
            window: Duration::from_secs(window_secs),
        }
    }

    #[test]
    fn allows_up_to_the_configured_maximum_per_key() {
        let limiter = RateLimiter::new(config(3, 60));
        let now = SystemTime::UNIX_EPOCH;

        assert!(limiter.check("k", now).is_ok());
        assert!(limiter.check("k", now).is_ok());
        assert!(limiter.check("k", now).is_ok());
    }

    #[test]
    fn rejects_the_request_past_the_maximum() {
        let limiter = RateLimiter::new(config(2, 60));
        let now = SystemTime::UNIX_EPOCH;

        assert!(limiter.check("k", now).is_ok());
        assert!(limiter.check("k", now).is_ok());
        let err = limiter.check("k", now).unwrap_err();
        assert_eq!(err.retry_after, Duration::from_secs(60));
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new(config(1, 60));
        let now = SystemTime::UNIX_EPOCH;

        assert!(limiter.check("a", now).is_ok());
        assert!(limiter.check("b", now).is_ok());
        assert!(limiter.check("a", now).is_err());
        assert!(limiter.check("b", now).is_err());
    }

    #[test]
    fn window_rollover_resets_the_count() {
        let limiter = RateLimiter::new(config(1, 60));
        let now = SystemTime::UNIX_EPOCH;

        assert!(limiter.check("k", now).is_ok());
        assert!(limiter.check("k", now + Duration::from_secs(59)).is_err());
        // The window has now fully elapsed: the count resets.
        assert!(limiter.check("k", now + Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn retry_after_reflects_remaining_time_in_the_window() {
        let limiter = RateLimiter::new(config(1, 60));
        let now = SystemTime::UNIX_EPOCH;

        limiter.check("k", now).unwrap();
        let err = limiter
            .check("k", now + Duration::from_secs(20))
            .unwrap_err();
        assert_eq!(err.retry_after, Duration::from_secs(40));
    }
}
