//! Postgres adapter for the [`AuditRepository`] port, over the
//! `admin_audit_events` table.
//!
//! Timestamps cross the boundary as Unix epoch seconds, the same convention
//! [`crate::admin_repo`] uses.

use domain::{
    AdminId, AuditEvent, AuditEventType, AuditId, AuditRepository, NewAuditEvent, RepositoryError,
};
use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::admin_repo::{from_epoch, to_epoch};

/// Map an arbitrary sqlx error to a backend [`RepositoryError`].
fn backend(err: sqlx::Error) -> RepositoryError {
    RepositoryError::Backend(err.to_string())
}

/// A Postgres-backed [`AuditRepository`].
#[derive(Debug, Clone)]
pub struct PgAuditRepository {
    pool: PgPool,
}

impl PgAuditRepository {
    /// Build the repository over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl AuditRepository for PgAuditRepository {
    async fn record(&self, event: &NewAuditEvent) -> Result<AuditId, RepositoryError> {
        let row = sqlx::query(
            "INSERT INTO admin_audit_events \
             (event_type, admin_id, email_attempted, ip, user_agent, occurred_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, to_timestamp($6)) \
             RETURNING id::text AS id",
        )
        .bind(event.event_type.as_str())
        .bind(event.admin_id.as_ref().map(AdminId::as_str))
        .bind(&event.email_attempted)
        .bind(&event.ip)
        .bind(&event.user_agent)
        .bind(to_epoch(event.occurred_at))
        .fetch_one(&self.pool)
        .await
        .map_err(backend)?;

        let id: String = row.try_get("id").map_err(backend)?;
        Ok(AuditId::new(id))
    }

    async fn list_recent(&self, limit: u32) -> Result<Vec<AuditEvent>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id::text AS id, event_type, admin_id::text AS admin_id, email_attempted, \
             ip, user_agent, EXTRACT(EPOCH FROM occurred_at)::bigint AS occurred_at_epoch \
             FROM admin_audit_events ORDER BY occurred_at DESC LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id").map_err(backend)?;
                let event_type: String = row.try_get("event_type").map_err(backend)?;
                let admin_id: Option<String> = row.try_get("admin_id").map_err(backend)?;
                let email_attempted: Option<String> =
                    row.try_get("email_attempted").map_err(backend)?;
                let ip: String = row.try_get("ip").map_err(backend)?;
                let user_agent: Option<String> = row.try_get("user_agent").map_err(backend)?;
                let occurred_at: i64 = row.try_get("occurred_at_epoch").map_err(backend)?;

                let event_type = AuditEventType::parse(&event_type)
                    .map_err(|e| RepositoryError::Backend(format!("stored event_type: {e}")))?;

                Ok(AuditEvent {
                    id: AuditId::new(id),
                    event_type,
                    admin_id: admin_id.map(AdminId::new),
                    email_attempted,
                    ip,
                    user_agent,
                    occurred_at: from_epoch(occurred_at),
                })
            })
            .collect()
    }
}
