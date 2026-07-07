//! Postgres adapters for the OAuth ports: [`PgPendingAuthStore`] over
//! `oauth_pending_authorizations` and [`PgOAuthIdentityRepository`] over
//! `admin_oauth_identities`.
//!
//! Timestamps cross the boundary as Unix epoch seconds, the same convention
//! [`crate::admin_repo`] uses.

use async_trait::async_trait;
use domain::{
    AdminId, OAuthIdentityRepository, PendingAuthStore, PendingAuthorization, ProviderId,
    RepositoryError,
};
use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::admin_repo::{from_epoch, to_epoch};

fn backend(err: sqlx::Error) -> RepositoryError {
    RepositoryError::Backend(err.to_string())
}

/// A Postgres-backed [`PendingAuthStore`].
#[derive(Debug, Clone)]
pub struct PgPendingAuthStore {
    pool: PgPool,
}

impl PgPendingAuthStore {
    /// Build the store over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PendingAuthStore for PgPendingAuthStore {
    async fn insert(&self, pending: &PendingAuthorization) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO oauth_pending_authorizations \
             (state, provider, nonce, code_verifier, redirect_uri, created_at) \
             VALUES ($1, $2, $3, $4, $5, to_timestamp($6))",
        )
        .bind(&pending.state)
        .bind(pending.provider.as_str())
        .bind(&pending.nonce)
        .bind(&pending.code_verifier)
        .bind(&pending.redirect_uri)
        .bind(to_epoch(pending.created_at))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn consume(&self, state: &str) -> Result<Option<PendingAuthorization>, RepositoryError> {
        // Fetch-and-delete in one statement, so a `state` cannot be replayed:
        // the second caller's DELETE ... RETURNING matches no row.
        let row = sqlx::query(
            "DELETE FROM oauth_pending_authorizations WHERE state = $1 \
             RETURNING state, provider, nonce, code_verifier, redirect_uri, \
             EXTRACT(EPOCH FROM created_at)::bigint AS created_at_epoch",
        )
        .bind(state)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let state: String = row.try_get("state").map_err(backend)?;
        let provider: String = row.try_get("provider").map_err(backend)?;
        let nonce: String = row.try_get("nonce").map_err(backend)?;
        let code_verifier: String = row.try_get("code_verifier").map_err(backend)?;
        let redirect_uri: String = row.try_get("redirect_uri").map_err(backend)?;
        let created_at: i64 = row.try_get("created_at_epoch").map_err(backend)?;

        let provider = ProviderId::parse(&provider)
            .map_err(|e| RepositoryError::Backend(format!("stored provider {provider:?}: {e}")))?;

        Ok(Some(PendingAuthorization {
            state,
            provider,
            nonce,
            code_verifier,
            redirect_uri,
            created_at: from_epoch(created_at),
        }))
    }
}

/// A Postgres-backed [`OAuthIdentityRepository`].
#[derive(Debug, Clone)]
pub struct PgOAuthIdentityRepository {
    pool: PgPool,
}

impl PgOAuthIdentityRepository {
    /// Build the repository over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OAuthIdentityRepository for PgOAuthIdentityRepository {
    async fn find_admin(
        &self,
        provider: &ProviderId,
        subject: &str,
    ) -> Result<Option<AdminId>, RepositoryError> {
        let row = sqlx::query(
            "SELECT admin_id::text AS admin_id FROM admin_oauth_identities \
             WHERE provider = $1 AND subject = $2",
        )
        .bind(provider.as_str())
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        match row {
            Some(row) => {
                let admin_id: String = row.try_get("admin_id").map_err(backend)?;
                Ok(Some(AdminId::new(admin_id)))
            }
            None => Ok(None),
        }
    }

    async fn link(
        &self,
        provider: &ProviderId,
        subject: &str,
        email: &str,
        admin_id: &AdminId,
    ) -> Result<(), RepositoryError> {
        // Idempotent: re-linking the same identity to the same admin refreshes
        // the stored email rather than erroring.
        sqlx::query(
            "INSERT INTO admin_oauth_identities (provider, subject, admin_id, email) \
             VALUES ($1, $2, $3::uuid, $4) \
             ON CONFLICT (provider, subject) DO UPDATE SET email = EXCLUDED.email",
        )
        .bind(provider.as_str())
        .bind(subject)
        .bind(admin_id.as_str())
        .bind(email)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }
}
