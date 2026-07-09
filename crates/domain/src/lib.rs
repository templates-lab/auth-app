//! Domain layer — the core of the hexagonal architecture.
//!
//! This crate holds the business model and the *ports* (traits) the outer
//! layers implement or consume. It depends on nothing but the standard
//! library: no web framework, no database driver. That purity is what the
//! dependency direction protects — `api → application → domain` and
//! `infrastructure → domain` both point inward, so the domain never reaches
//! outward to a framework.

pub mod audit;
pub mod auth;
pub mod health;
pub mod oauth;
pub mod saro;
pub mod session;

pub use audit::{
    AuditEvent, AuditEventType, AuditEventTypeError, AuditId, AuditRepository, NewAuditEvent,
};
pub use auth::{
    AdminAccount, AdminId, AdminRepository, Clock, Email, EmailError, IpLockoutStore,
    LockoutPolicy, LockoutState, NewAdmin, PasswordHash, PasswordHashError, PasswordHasher,
    PasswordPolicy, PasswordPolicyError, PasswordRequirement, RepositoryError, Role, RoleError,
};
pub use health::{Health, HealthCheck, Readiness};
pub use oauth::{
    AuthorizeParams, ExchangeRequest, OAuthError, OAuthIdentity, OAuthIdentityRepository,
    OAuthProvider, OAuthSecretGenerator, PendingAuthStore, PendingAuthorization, PkcePair,
    ProviderId,
};
pub use session::{
    CsrfToken, Session, SessionPolicy, SessionRepository, SessionToken, SessionTokenGenerator,
};
