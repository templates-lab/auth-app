//! Infrastructure layer — adapters that implement domain ports.
//!
//! Depends on `domain` (for the port contracts) and on external drivers
//! (`sqlx` for Postgres). It never depends on `application` or `api`:
//! dependencies point inward, toward the domain.

use async_trait::async_trait;
use domain::{Health, HealthCheck, Readiness};

pub mod admin_repo;
pub mod audit_repo;
pub mod clock;
pub mod db;
pub mod health;
pub mod oauth_http;
pub mod oauth_provider;
pub mod oauth_repos;
pub mod oauth_secrets;
pub mod password;
pub mod payments_repo;
pub mod session_repo;
pub mod tokens;

pub use admin_repo::{PgAdminRepository, PgIpLockoutStore};
pub use audit_repo::PgAuditRepository;
pub use clock::SystemClock;
pub use db::{connect, run_migrations, PgConfig, PgConfigError, MIGRATOR};
pub use health::PgHealthCheck;
pub use oauth_http::{HttpClient, ReqwestHttpClient};
pub use oauth_provider::{OidcConfig, OidcProvider};
pub use oauth_repos::{PgOAuthIdentityRepository, PgPendingAuthStore};
pub use oauth_secrets::OAuthSecrets;
pub use password::{Argon2Hasher, Argon2Params};
pub use payments_repo::PgPaymentRepository;
pub use session_repo::PgSessionRepository;
pub use tokens::SecureRandomTokens;

/// A trivial [`HealthCheck`] adapter that always reports the service ready.
///
/// A null adapter for tests and for delivery paths with no database dependency;
/// production wiring uses [`PgHealthCheck`], which probes the live pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysReady;

#[async_trait]
impl HealthCheck for AlwaysReady {
    async fn check(&self) -> Health {
        Health {
            readiness: Readiness::Ready,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn always_ready_reports_ready() {
        assert_eq!(AlwaysReady.check().await.readiness, Readiness::Ready);
    }
}
