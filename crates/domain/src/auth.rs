//! Admin authentication domain: the model and ports for logging an
//! administrator in with an email and password.
//!
//! Following the hexagonal discipline of this crate, everything here is pure:
//! value objects ([`Email`], [`PasswordPolicy`], [`PasswordHash`],
//! [`LockoutPolicy`], [`LockoutState`]) and *ports* (traits) the outer layers
//! implement — [`PasswordHasher`], [`AdminRepository`], [`IpLockoutStore`],
//! [`Clock`]. The domain declares *what* a password hasher or a repository must
//! do; the `infrastructure` crate decides *how* (argon2, Postgres). No web
//! framework, no database driver, no clock reaches in here.

use std::time::{Duration, SystemTime};

/// A normalized administrator email address.
///
/// Construction normalizes (trims surrounding whitespace and lowercases) and
/// performs a deliberately minimal structural check — a single `@` with a
/// non-empty local part and a dotted, non-empty domain. Deep RFC-5322
/// validation is intentionally out of scope: the goal is a stable lookup key,
/// not a deliverability guarantee. Because lookups compare the normalized form,
/// `Admin@Example.com ` and `admin@example.com` are the same account.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    /// Parse and normalize an email address.
    pub fn parse(raw: &str) -> Result<Self, EmailError> {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Err(EmailError::Empty);
        }

        let (local, domain) = normalized.split_once('@').ok_or(EmailError::MissingAt)?;
        // A second `@`, an empty local part, or a domain without a dot (or with
        // empty labels) is rejected — enough to catch fat-fingered input without
        // pretending to be a full RFC validator.
        let domain_ok = domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.');
        if local.is_empty() || domain.contains('@') || !domain_ok {
            return Err(EmailError::Malformed);
        }

        Ok(Self(normalized))
    }

    /// The normalized address as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Email {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why an email failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailError {
    /// The input was empty after trimming.
    Empty,
    /// The input had no `@` separator.
    MissingAt,
    /// The input had an `@` but was otherwise structurally invalid.
    Malformed,
}

impl std::fmt::Display for EmailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("email must not be empty"),
            Self::MissingAt => f.write_str("email must contain '@'"),
            Self::Malformed => f.write_str("email is malformed"),
        }
    }
}

impl std::error::Error for EmailError {}

/// An opaque password hash — the encoded output of a [`PasswordHasher`].
///
/// The domain treats it as an opaque token: it is produced and verified only
/// through the [`PasswordHasher`] port, never inspected here. The concrete
/// adapter (argon2) stores its parameters and salt inside the encoded string
/// (the PHC format), which is exactly why verification needs no side channel.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// Wrap an already-encoded hash string produced by a [`PasswordHasher`] (or
    /// loaded from storage).
    pub fn from_encoded(encoded: impl Into<String>) -> Self {
        Self(encoded.into())
    }

    /// The encoded hash string, for persistence.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// A hash is a secret-adjacent value; keep it out of logs by not printing its
// contents. The workspace's `missing_debug_implementations` lint still wants a
// `Debug` impl, so provide a redacting one.
impl std::fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordHash(<redacted>)")
    }
}

/// A configurable password-strength policy.
///
/// Every rule is a field, so the composition root builds the policy from
/// configuration (environment) rather than hard-coding thresholds. The defaults
/// ([`PasswordPolicy::recommended`]) follow common guidance: a 12-character
/// floor with mixed character classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordPolicy {
    /// Minimum length, counted in Unicode scalar values.
    pub min_length: usize,
    /// Require at least one uppercase letter.
    pub require_uppercase: bool,
    /// Require at least one lowercase letter.
    pub require_lowercase: bool,
    /// Require at least one ASCII digit.
    pub require_digit: bool,
    /// Require at least one non-alphanumeric character.
    pub require_symbol: bool,
}

impl PasswordPolicy {
    /// A sensible default: 12+ chars with upper, lower, and digit required.
    pub const fn recommended() -> Self {
        Self {
            min_length: 12,
            require_uppercase: true,
            require_lowercase: true,
            require_digit: true,
            require_symbol: false,
        }
    }

    /// Check a candidate password against every enabled rule.
    ///
    /// Returns every unmet requirement (not just the first), so a bootstrap tool
    /// can tell the operator all the ways their password fell short in one go.
    pub fn validate(&self, password: &str) -> Result<(), PasswordPolicyError> {
        let mut unmet = Vec::new();

        if password.chars().count() < self.min_length {
            unmet.push(PasswordRequirement::MinLength(self.min_length));
        }
        if self.require_uppercase && !password.chars().any(|c| c.is_ascii_uppercase()) {
            unmet.push(PasswordRequirement::Uppercase);
        }
        if self.require_lowercase && !password.chars().any(|c| c.is_ascii_lowercase()) {
            unmet.push(PasswordRequirement::Lowercase);
        }
        if self.require_digit && !password.chars().any(|c| c.is_ascii_digit()) {
            unmet.push(PasswordRequirement::Digit);
        }
        if self.require_symbol && !password.chars().any(|c| !c.is_alphanumeric()) {
            unmet.push(PasswordRequirement::Symbol);
        }

        if unmet.is_empty() {
            Ok(())
        } else {
            Err(PasswordPolicyError { unmet })
        }
    }
}

/// A single password rule that a candidate failed to satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordRequirement {
    /// The password was shorter than the configured minimum.
    MinLength(usize),
    /// An uppercase letter was required but missing.
    Uppercase,
    /// A lowercase letter was required but missing.
    Lowercase,
    /// A digit was required but missing.
    Digit,
    /// A symbol (non-alphanumeric) was required but missing.
    Symbol,
}

impl std::fmt::Display for PasswordRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MinLength(n) => write!(f, "at least {n} characters"),
            Self::Uppercase => f.write_str("an uppercase letter"),
            Self::Lowercase => f.write_str("a lowercase letter"),
            Self::Digit => f.write_str("a digit"),
            Self::Symbol => f.write_str("a symbol"),
        }
    }
}

/// One or more unmet password requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordPolicyError {
    /// Every requirement the candidate password failed to meet.
    pub unmet: Vec<PasswordRequirement>,
}

impl std::fmt::Display for PasswordPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("password does not meet policy: needs ")?;
        for (i, req) in self.unmet.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{req}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PasswordPolicyError {}

/// The mutable lockout counters for one principal (an account or a client IP).
///
/// It is a plain value object: the transition rules live in [`LockoutPolicy`],
/// and persistence lives behind the [`AdminRepository`] / [`IpLockoutStore`]
/// ports. Keeping the state dumb and the policy pure makes the whole lockout
/// behavior unit-testable without a database or a real clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LockoutState {
    /// Consecutive failed attempts since the last success.
    pub failed_attempts: u32,
    /// When the principal is locked until, if currently locked.
    pub locked_until: Option<SystemTime>,
}

impl LockoutState {
    /// The cleared state: no failures, not locked. Used after a success.
    pub const fn clear() -> Self {
        Self {
            failed_attempts: 0,
            locked_until: None,
        }
    }
}

/// A progressive lockout policy shared by account- and IP-level throttling.
///
/// Failures accumulate; once they reach [`Self::max_attempts`] the principal is
/// locked for a delay that grows with each further failure — doubling from
/// [`Self::base_delay`] and capped at [`Self::max_delay`]. The same pure rules
/// drive both the per-account lock (stored on the admin row) and the per-IP lock
/// (stored keyed by address), so an attacker is throttled whether they spray one
/// account from many IPs or many accounts from one IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockoutPolicy {
    /// Failures tolerated before the first lock engages.
    pub max_attempts: u32,
    /// The lock duration applied at the threshold; doubles per extra failure.
    pub base_delay: Duration,
    /// The ceiling the doubling delay is clamped to.
    pub max_delay: Duration,
}

impl LockoutPolicy {
    /// A sensible default: lock after 5 failures for 1 minute, up to 1 hour.
    pub const fn recommended() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(3600),
        }
    }

    /// Whether `state` is locked at instant `now`.
    pub fn is_locked(&self, state: &LockoutState, now: SystemTime) -> bool {
        matches!(state.locked_until, Some(until) if until > now)
    }

    /// The delay remaining until `state` unlocks at `now`, if locked.
    pub fn retry_after(&self, state: &LockoutState, now: SystemTime) -> Option<Duration> {
        match state.locked_until {
            Some(until) => until.duration_since(now).ok().filter(|d| !d.is_zero()),
            None => None,
        }
    }

    /// Compute the next state after a failed attempt at instant `now`.
    ///
    /// The counter increments; once it reaches `max_attempts` the principal is
    /// locked from `now` for a delay that doubles with each failure past the
    /// threshold, clamped to `max_delay`.
    pub fn register_failure(&self, state: &LockoutState, now: SystemTime) -> LockoutState {
        let failed_attempts = state.failed_attempts.saturating_add(1);
        let locked_until = if failed_attempts >= self.max_attempts.max(1) {
            Some(now + self.lock_duration(failed_attempts))
        } else {
            None
        };
        LockoutState {
            failed_attempts,
            locked_until,
        }
    }

    /// The lock duration for a given failure count: `base * 2^(n - max)` capped
    /// at `max_delay`. At exactly the threshold the delay is `base_delay`.
    fn lock_duration(&self, failed_attempts: u32) -> Duration {
        let threshold = self.max_attempts.max(1);
        let over = failed_attempts.saturating_sub(threshold);
        // Cap the shift so `2^over` cannot overflow; anything beyond saturates to
        // `max_delay` anyway.
        let factor = 1u64.checked_shl(over.min(32)).unwrap_or(u64::MAX);
        let scaled = self
            .base_delay
            .checked_mul(factor.min(u32::MAX as u64) as u32)
            .unwrap_or(self.max_delay);
        scaled.min(self.max_delay)
    }
}

/// A stored administrator account, as the repository hands it to the domain.
#[derive(Debug, Clone)]
pub struct AdminAccount {
    /// Opaque unique identifier (the database primary key, as text).
    pub id: AdminId,
    /// The account's normalized email.
    pub email: Email,
    /// The stored password hash to verify against.
    pub password_hash: PasswordHash,
    /// The account's current lockout counters.
    pub lockout: LockoutState,
}

/// An opaque administrator identifier.
///
/// Held as text so the domain need not depend on a UUID library; the
/// infrastructure adapter maps it to/from the database's `uuid` column.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AdminId(String);

impl AdminId {
    /// Wrap an identifier string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AdminId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A new administrator to persist (used by the bootstrap use case).
#[derive(Debug, Clone)]
pub struct NewAdmin {
    /// The account's normalized email.
    pub email: Email,
    /// The already-hashed password.
    pub password_hash: PasswordHash,
}

/// Port: hashes and verifies passwords.
///
/// Implemented by an argon2 adapter in `infrastructure`. Hashing and
/// verification are `async` so the adapter can offload the CPU-bound work to a
/// blocking thread pool without blocking the async runtime.
#[async_trait::async_trait]
pub trait PasswordHasher: Send + Sync {
    /// Hash a plaintext password, returning the encoded [`PasswordHash`].
    async fn hash(&self, plaintext: &str) -> Result<PasswordHash, PasswordHashError>;

    /// Verify a plaintext password against an encoded hash.
    ///
    /// Returns `Ok(true)` on a match, `Ok(false)` on a mismatch, and `Err` only
    /// when the hash could not be evaluated (e.g. a corrupt encoding).
    async fn verify(&self, plaintext: &str, hash: &PasswordHash)
        -> Result<bool, PasswordHashError>;

    /// A valid reference hash with the adapter's own parameters.
    ///
    /// The login use case verifies against this when no account matches, so a
    /// request for a nonexistent user performs the same argon2 work — and takes
    /// the same time — as one for a real user with a wrong password. This is the
    /// timing-equalization that stops attackers from enumerating accounts.
    fn reference_hash(&self) -> PasswordHash;
}

/// A password hashing/verification failure (not a mismatch — see
/// [`PasswordHasher::verify`]).
#[derive(Debug)]
pub struct PasswordHashError(pub String);

impl std::fmt::Display for PasswordHashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "password hashing failed: {}", self.0)
    }
}

impl std::error::Error for PasswordHashError {}

/// Port: persistence for administrator accounts.
///
/// Implemented by a Postgres adapter in `infrastructure`. The domain names the
/// operations the login and bootstrap use cases need; the adapter owns the SQL.
#[async_trait::async_trait]
pub trait AdminRepository: Send + Sync {
    /// Find an account by its normalized email, if one exists.
    async fn find_by_email(&self, email: &Email) -> Result<Option<AdminAccount>, RepositoryError>;

    /// Overwrite an account's lockout counters.
    async fn update_lockout(
        &self,
        id: &AdminId,
        state: &LockoutState,
    ) -> Result<(), RepositoryError>;

    /// Number of administrator accounts (used to gate first-admin bootstrap).
    async fn count(&self) -> Result<u64, RepositoryError>;

    /// Insert a new administrator, returning its assigned id.
    ///
    /// Implementations reject a duplicate email with
    /// [`RepositoryError::EmailTaken`] rather than a generic error, so the
    /// bootstrap use case can report it precisely.
    async fn insert(&self, admin: &NewAdmin) -> Result<AdminId, RepositoryError>;
}

/// Port: persistence for per-IP lockout counters.
///
/// Separate from [`AdminRepository`] because an IP is not an account: it is
/// throttled even when it targets emails that do not exist, which is exactly
/// what defeats account-enumeration-by-lockout.
#[async_trait::async_trait]
pub trait IpLockoutStore: Send + Sync {
    /// The current lockout counters for `ip` (cleared state if never seen).
    async fn get(&self, ip: &str) -> Result<LockoutState, RepositoryError>;

    /// Overwrite the lockout counters for `ip`.
    async fn put(&self, ip: &str, state: &LockoutState) -> Result<(), RepositoryError>;
}

/// A storage failure from a repository port.
#[derive(Debug)]
pub enum RepositoryError {
    /// An insert was rejected because the email already exists.
    EmailTaken,
    /// Any other backend failure, described for logs.
    Backend(String),
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmailTaken => f.write_str("an admin with that email already exists"),
            Self::Backend(msg) => write!(f, "repository backend error: {msg}"),
        }
    }
}

impl std::error::Error for RepositoryError {}

/// Port: the current wall-clock time.
///
/// Injected so lockout timing is deterministic under test — a fake clock lets a
/// test advance time past a lock without sleeping.
pub trait Clock: Send + Sync {
    /// The current instant.
    fn now(&self) -> SystemTime;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalizes_and_validates() {
        assert_eq!(
            Email::parse("  Admin@Example.COM ").unwrap().as_str(),
            "admin@example.com"
        );
        assert_eq!(Email::parse("").unwrap_err(), EmailError::Empty);
        assert_eq!(
            Email::parse("no-at-sign").unwrap_err(),
            EmailError::MissingAt
        );
        assert_eq!(Email::parse("a@b").unwrap_err(), EmailError::Malformed);
        assert_eq!(
            Email::parse("@example.com").unwrap_err(),
            EmailError::Malformed
        );
    }

    #[test]
    fn password_policy_reports_every_unmet_requirement() {
        let policy = PasswordPolicy::recommended();
        let err = policy.validate("short").unwrap_err();
        assert!(err.unmet.contains(&PasswordRequirement::MinLength(12)));
        assert!(err.unmet.contains(&PasswordRequirement::Uppercase));
        assert!(err.unmet.contains(&PasswordRequirement::Digit));

        assert!(policy.validate("Str0ngEnoughPass").is_ok());
    }

    #[test]
    fn symbol_requirement_is_configurable() {
        let policy = PasswordPolicy {
            require_symbol: true,
            ..PasswordPolicy::recommended()
        };
        assert!(policy
            .validate("Str0ngEnoughPass")
            .unwrap_err()
            .unmet
            .contains(&PasswordRequirement::Symbol));
        assert!(policy.validate("Str0ngEnough!Pass").is_ok());
    }

    #[test]
    fn lockout_engages_at_threshold_and_backs_off_progressively() {
        let policy = LockoutPolicy {
            max_attempts: 3,
            base_delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(80),
        };
        let now = SystemTime::UNIX_EPOCH;

        // Below the threshold: counting up, not yet locked.
        let s1 = policy.register_failure(&LockoutState::clear(), now);
        assert_eq!(s1.failed_attempts, 1);
        assert!(!policy.is_locked(&s1, now));
        let s2 = policy.register_failure(&s1, now);
        assert!(!policy.is_locked(&s2, now));

        // At the threshold: locked for base_delay.
        let s3 = policy.register_failure(&s2, now);
        assert_eq!(s3.failed_attempts, 3);
        assert!(policy.is_locked(&s3, now));
        assert_eq!(s3.locked_until, Some(now + Duration::from_secs(10)));
        assert!(!policy.is_locked(&s3, now + Duration::from_secs(10)));

        // Past the threshold: the delay doubles (20s), then clamps at max (80s).
        let s4 = policy.register_failure(&s3, now);
        assert_eq!(s4.locked_until, Some(now + Duration::from_secs(20)));
        let mut s = s4;
        for _ in 0..10 {
            s = policy.register_failure(&s, now);
        }
        assert_eq!(s.locked_until, Some(now + Duration::from_secs(80)));
    }

    #[test]
    fn retry_after_reports_remaining_lock() {
        let policy = LockoutPolicy::recommended();
        let now = SystemTime::UNIX_EPOCH;
        let locked = LockoutState {
            failed_attempts: 5,
            locked_until: Some(now + Duration::from_secs(30)),
        };
        assert_eq!(
            policy.retry_after(&locked, now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            policy.retry_after(&locked, now + Duration::from_secs(30)),
            None
        );
        assert_eq!(policy.retry_after(&LockoutState::clear(), now), None);
    }
}
