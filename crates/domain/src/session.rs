//! Server-side admin sessions: the model and ports for issuing, validating,
//! rotating, and revoking a session after a successful login.
//!
//! Following the same hexagonal discipline as [`crate::auth`], this module is
//! pure: value objects ([`SessionToken`], [`CsrfToken`], [`Session`],
//! [`SessionPolicy`]) and *ports* ([`SessionRepository`],
//! [`SessionTokenGenerator`]) the outer layers implement. The domain declares
//! *what* a session store or a token generator must do; `infrastructure`
//! decides *how* (Postgres, a CSPRNG). No cookie, no HTTP header, no database
//! driver reaches in here.

use std::time::{Duration, SystemTime};

use crate::{AdminId, RepositoryError, Role};

/// The secret bearer value handed to the client as the session cookie.
///
/// Opaque to everything but the [`SessionRepository`] adapter, which looks a
/// session up by this exact value. Cryptographically random and unguessable —
/// see [`SessionTokenGenerator`] for how it is produced.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SessionToken(String);

impl SessionToken {
    /// Wrap a raw token value — either freshly generated, or read back from an
    /// inbound cookie before looking it up.
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The raw token, for persistence or for writing into a cookie.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// A session token is a bearer secret; keep it out of logs the same way
// `PasswordHash` does.
impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

/// A CSRF token, verified via the synchronizer-token pattern: issued alongside
/// the session, mirrored into a client-readable cookie, and echoed back by the
/// caller in a request header on every mutation. [`Self::verify`] is what the
/// delivery layer calls to check that echo.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CsrfToken(String);

impl CsrfToken {
    /// Wrap a raw token value.
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The raw token, for persistence or for writing into a cookie.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compare against a candidate (e.g. the `X-CSRF-Token` header) in time
    /// proportional to the candidate's length, not short-circuiting on the
    /// first mismatched byte — so a network timing side-channel cannot help an
    /// attacker guess the token one byte at a time.
    pub fn verify(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let given = candidate.as_bytes();

        if expected.len() != given.len() {
            // Still do proportional work on the mismatched-length path so
            // "wrong length" and "wrong contents" cost the same.
            let mut diff = 0u8;
            for &byte in given {
                diff |= byte;
            }
            let _ = diff;
            return false;
        }

        let mut diff = 0u8;
        for (a, b) in expected.iter().zip(given.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

// Not a bearer secret in the same sense as `SessionToken` — it is deliberately
// readable by client-side script — but redact anyway so a stray `{:?}` in a log
// line never becomes a habit.
impl std::fmt::Debug for CsrfToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CsrfToken(<redacted>)")
    }
}

/// A stored session, as the repository hands it to the application layer.
#[derive(Debug, Clone)]
pub struct Session {
    /// The bearer token identifying this session (the row's lookup key).
    pub token: SessionToken,
    /// The authenticated administrator this session belongs to.
    pub admin_id: AdminId,
    /// The admin's role at the moment this session was issued — a snapshot,
    /// not a live lookup: a role change takes effect on that admin's next
    /// login, the same trade-off `csrf_token` and `admin_id` already make by
    /// living on the session rather than being re-fetched every request.
    pub role: Role,
    /// The CSRF token issued alongside this session.
    pub csrf_token: CsrfToken,
    /// When the session was created (or last rotated).
    pub created_at: SystemTime,
    /// The absolute deadline past which the session is dead no matter how
    /// recently it was used.
    pub absolute_expires_at: SystemTime,
    /// The last instant the session was successfully used; the idle-timeout
    /// clock measures from here.
    pub last_seen_at: SystemTime,
}

/// Configurable expiration rules: an absolute ceiling and a sliding idle
/// window. A session dies at whichever comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    /// How long a session may sit unused before it is considered dead.
    pub idle_timeout: Duration,
    /// The hard ceiling on a session's lifetime, regardless of activity.
    pub absolute_timeout: Duration,
}

impl SessionPolicy {
    /// A sensible default: 30 minutes idle, 12 hours absolute.
    pub const fn recommended() -> Self {
        Self {
            idle_timeout: Duration::from_secs(30 * 60),
            absolute_timeout: Duration::from_secs(12 * 60 * 60),
        }
    }

    /// Whether `session` is expired at instant `now` — by the absolute
    /// ceiling, by the idle window, or (defensively) by an unrepresentable
    /// `last_seen_at`/`now` pair.
    pub fn is_expired(&self, session: &Session, now: SystemTime) -> bool {
        if now >= session.absolute_expires_at {
            return true;
        }
        match now.duration_since(session.last_seen_at) {
            Ok(idle_for) => idle_for >= self.idle_timeout,
            // `now` before `last_seen_at` should not happen with a monotonic
            // clock; treat it as expired rather than trust a clock that went
            // backwards.
            Err(_) => true,
        }
    }
}

/// Port: persistence for sessions.
///
/// Implemented by a Postgres adapter in `infrastructure`. The domain names the
/// operations the session use cases need; the adapter owns the SQL.
#[async_trait::async_trait]
pub trait SessionRepository: Send + Sync {
    /// Persist a freshly issued session.
    async fn insert(&self, session: &Session) -> Result<(), RepositoryError>;

    /// Look a session up by its bearer token, if one exists.
    async fn find(&self, token: &SessionToken) -> Result<Option<Session>, RepositoryError>;

    /// Slide the idle-timeout window forward after a successful use.
    async fn touch(
        &self,
        token: &SessionToken,
        last_seen_at: SystemTime,
    ) -> Result<(), RepositoryError>;

    /// Remove a session — logout, rotation, or expiry cleanup.
    async fn delete(&self, token: &SessionToken) -> Result<(), RepositoryError>;
}

/// Port: generates the random values a session needs.
///
/// Implemented in `infrastructure` over a CSPRNG. Kept as a port (rather than
/// calling a random-number crate from the application layer) for the same
/// reason [`crate::auth::PasswordHasher`] is one: the domain and application
/// layers stay free of any concrete crypto dependency.
pub trait SessionTokenGenerator: Send + Sync {
    /// A fresh, unguessable session bearer token.
    fn generate_session_token(&self) -> SessionToken;

    /// A fresh, unguessable CSRF token.
    fn generate_csrf_token(&self) -> CsrfToken;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_token_verifies_exact_match_only() {
        let token = CsrfToken::from_raw("abc123");
        assert!(token.verify("abc123"));
        assert!(!token.verify("abc124"));
        assert!(!token.verify("abc12"));
        assert!(!token.verify(""));
    }

    #[test]
    fn session_expires_at_absolute_ceiling_even_if_recently_used() {
        let now = SystemTime::UNIX_EPOCH;
        let policy = SessionPolicy {
            idle_timeout: Duration::from_secs(3600),
            absolute_timeout: Duration::from_secs(60),
        };
        let session = Session {
            token: SessionToken::from_raw("t"),
            admin_id: AdminId::new("a"),
            role: Role::admin(),
            csrf_token: CsrfToken::from_raw("c"),
            created_at: now,
            absolute_expires_at: now + Duration::from_secs(60),
            last_seen_at: now + Duration::from_secs(59),
        };
        assert!(!policy.is_expired(&session, now + Duration::from_secs(59)));
        assert!(policy.is_expired(&session, now + Duration::from_secs(60)));
    }

    #[test]
    fn session_expires_after_idle_window_even_within_absolute_ceiling() {
        let now = SystemTime::UNIX_EPOCH;
        let policy = SessionPolicy {
            idle_timeout: Duration::from_secs(30),
            absolute_timeout: Duration::from_secs(3600),
        };
        let session = Session {
            token: SessionToken::from_raw("t"),
            admin_id: AdminId::new("a"),
            role: Role::admin(),
            csrf_token: CsrfToken::from_raw("c"),
            created_at: now,
            absolute_expires_at: now + Duration::from_secs(3600),
            last_seen_at: now,
        };
        assert!(!policy.is_expired(&session, now + Duration::from_secs(29)));
        assert!(policy.is_expired(&session, now + Duration::from_secs(30)));
    }
}
