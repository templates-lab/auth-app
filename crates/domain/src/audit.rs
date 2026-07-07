//! Authentication audit trail: the model and port for recording — and later
//! querying — every significant auth event.
//!
//! Following the same hexagonal discipline as [`crate::auth`] and
//! [`crate::session`]: pure value objects and a port ([`AuditRepository`]) the
//! `infrastructure` layer implements. No event ever carries a password or a
//! session/CSRF token — see [`NewAuditEvent`] for exactly what a record may
//! hold.

use std::time::SystemTime;

use crate::AdminId;

/// An opaque identifier for a stored audit event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AuditId(String);

impl AuditId {
    /// Wrap an identifier string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AuditId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The kind of event recorded.
///
/// A closed set, matching exactly the events this codebase can actually emit
/// today — `login_succeeded`/`login_failed`/`locked_out` (from
/// [`crate::auth::LoginService`]) and `logged_out` (from
/// [`crate::session::SessionRepository::delete`]'s caller). Refresh, OAuth
/// account linking, and password changes join this set once those features
/// exist; adding a variant here is the only change their audit hook needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditEventType {
    /// A login attempt succeeded.
    LoginSucceeded,
    /// A login attempt failed — wrong password or no such account
    /// (indistinguishable by design; see [`crate::auth::LoginError`]).
    LoginFailed,
    /// A login attempt was rejected because the account or IP is locked out.
    LockedOut,
    /// A session was revoked via logout.
    LoggedOut,
}

impl AuditEventType {
    /// The stable string form persisted to storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LoginSucceeded => "login_succeeded",
            Self::LoginFailed => "login_failed",
            Self::LockedOut => "locked_out",
            Self::LoggedOut => "logged_out",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, AuditEventTypeError> {
        Ok(match raw {
            "login_succeeded" => Self::LoginSucceeded,
            "login_failed" => Self::LoginFailed,
            "locked_out" => Self::LockedOut,
            "logged_out" => Self::LoggedOut,
            other => return Err(AuditEventTypeError::Unknown(other.to_string())),
        })
    }
}

impl std::fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stored audit event's type string did not match any known
/// [`AuditEventType`] — a data-integrity fault, since every write goes
/// through [`AuditEventType::as_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEventTypeError {
    /// The raw value read back from storage.
    Unknown(String),
}

impl std::fmt::Display for AuditEventTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(raw) => write!(f, "unknown audit event type {raw:?}"),
        }
    }
}

impl std::error::Error for AuditEventTypeError {}

/// A stored audit event, as the repository hands it back for display.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// The event's own id.
    pub id: AuditId,
    /// What happened.
    pub event_type: AuditEventType,
    /// The admin this event concerns, once one is known. `None` for a login
    /// failure against an email that matches no account.
    pub admin_id: Option<AdminId>,
    /// The email address submitted, for login events — kept even when it
    /// matched no account, since that is exactly the case `admin_id` cannot
    /// identify. Never a password.
    pub email_attempted: Option<String>,
    /// The client IP the request arrived from.
    pub ip: String,
    /// The client's `User-Agent` header, if it sent one.
    pub user_agent: Option<String>,
    /// When the event happened.
    pub occurred_at: SystemTime,
}

/// A new audit event to persist.
///
/// Deliberately has no field for a password, a session token, or a CSRF
/// token — there is nowhere in this type to put one, which is what makes "no
/// sensitive data in the audit trail" a property of the type rather than a
/// convention callers must remember.
#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    /// What happened.
    pub event_type: AuditEventType,
    /// The admin this event concerns, once one is known.
    pub admin_id: Option<AdminId>,
    /// The email address submitted, for login events.
    pub email_attempted: Option<String>,
    /// The client IP the request arrived from.
    pub ip: String,
    /// The client's `User-Agent` header, if it sent one.
    pub user_agent: Option<String>,
    /// When the event happened.
    pub occurred_at: SystemTime,
}

/// Port: persistence for the audit trail.
///
/// Implemented by a Postgres adapter in `infrastructure`. Recording is
/// best-effort from the caller's point of view — a delivery-layer caller logs
/// (rather than fails the request) if `record` itself errors, since an outage
/// in the audit store must never block a real login or logout.
#[async_trait::async_trait]
pub trait AuditRepository: Send + Sync {
    /// Persist a new audit event.
    async fn record(&self, event: &NewAuditEvent) -> Result<AuditId, crate::RepositoryError>;

    /// The most recent events, newest first, capped at `limit`.
    async fn list_recent(&self, limit: u32) -> Result<Vec<AuditEvent>, crate::RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_round_trips_through_its_string_form() {
        for event_type in [
            AuditEventType::LoginSucceeded,
            AuditEventType::LoginFailed,
            AuditEventType::LockedOut,
            AuditEventType::LoggedOut,
        ] {
            assert_eq!(
                AuditEventType::parse(event_type.as_str()).unwrap(),
                event_type
            );
        }
    }

    #[test]
    fn unknown_event_type_string_is_rejected() {
        assert!(matches!(
            AuditEventType::parse("not_a_real_event"),
            Err(AuditEventTypeError::Unknown(_))
        ));
    }
}
