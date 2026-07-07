//! Postgres adapter for the [`SessionRepository`] port, over the
//! `admin_sessions` table.
//!
//! Timestamps cross the boundary as Unix epoch seconds, the same convention
//! [`crate::admin_repo`] uses and for the same reason: it keeps the adapter to
//! sqlx's built-in scalar types with no timezone-typed column binding, while
//! the column itself stays a proper `TIMESTAMPTZ`.

use std::time::SystemTime;

use async_trait::async_trait;
use domain::{AdminId, CsrfToken, RepositoryError, Session, SessionRepository, SessionToken};
use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::admin_repo::{from_epoch, to_epoch};

/// Map an arbitrary sqlx error to a backend [`RepositoryError`].
fn backend(err: sqlx::Error) -> RepositoryError {
    RepositoryError::Backend(err.to_string())
}

/// A Postgres-backed [`SessionRepository`].
#[derive(Debug, Clone)]
pub struct PgSessionRepository {
    pool: PgPool,
}

impl PgSessionRepository {
    /// Build the repository over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn insert(&self, session: &Session) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO admin_sessions \
             (token, admin_id, csrf_token, created_at, last_seen_at, absolute_expires_at) \
             VALUES ($1, $2::uuid, $3, to_timestamp($4), to_timestamp($5), to_timestamp($6))",
        )
        .bind(session.token.as_str())
        .bind(session.admin_id.as_str())
        .bind(session.csrf_token.as_str())
        .bind(to_epoch(session.created_at))
        .bind(to_epoch(session.last_seen_at))
        .bind(to_epoch(session.absolute_expires_at))
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn find(&self, token: &SessionToken) -> Result<Option<Session>, RepositoryError> {
        let row = sqlx::query(
            "SELECT token, admin_id::text AS admin_id, csrf_token, \
             EXTRACT(EPOCH FROM created_at)::bigint AS created_at_epoch, \
             EXTRACT(EPOCH FROM last_seen_at)::bigint AS last_seen_at_epoch, \
             EXTRACT(EPOCH FROM absolute_expires_at)::bigint AS absolute_expires_at_epoch \
             FROM admin_sessions WHERE token = $1",
        )
        .bind(token.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let token_str: String = row.try_get("token").map_err(backend)?;
        let admin_id: String = row.try_get("admin_id").map_err(backend)?;
        let csrf_token: String = row.try_get("csrf_token").map_err(backend)?;
        let created_at: i64 = row.try_get("created_at_epoch").map_err(backend)?;
        let last_seen_at: i64 = row.try_get("last_seen_at_epoch").map_err(backend)?;
        let absolute_expires_at: i64 = row.try_get("absolute_expires_at_epoch").map_err(backend)?;

        Ok(Some(Session {
            token: SessionToken::from_raw(token_str),
            admin_id: AdminId::new(admin_id),
            csrf_token: CsrfToken::from_raw(csrf_token),
            created_at: from_epoch(created_at),
            last_seen_at: from_epoch(last_seen_at),
            absolute_expires_at: from_epoch(absolute_expires_at),
        }))
    }

    async fn touch(
        &self,
        token: &SessionToken,
        last_seen_at: SystemTime,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE admin_sessions SET last_seen_at = to_timestamp($2) WHERE token = $1")
            .bind(token.as_str())
            .bind(to_epoch(last_seen_at))
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn delete(&self, token: &SessionToken) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM admin_sessions WHERE token = $1")
            .bind(token.as_str())
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }
}
