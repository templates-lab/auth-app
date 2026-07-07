//! Postgres adapter for the [`WebhookEventStore`] port, over
//! `payments.webhook_events`.

use async_trait::async_trait;
use payments::{WebhookEventStore, WebhookStoreError};
use sqlx::postgres::PgPool;
use sqlx::Row;

fn backend(err: sqlx::Error) -> WebhookStoreError {
    WebhookStoreError(err.to_string())
}

/// A Postgres-backed [`WebhookEventStore`].
#[derive(Debug, Clone)]
pub struct PgWebhookEventStore {
    pool: PgPool,
}

impl PgWebhookEventStore {
    /// Build the store over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WebhookEventStore for PgWebhookEventStore {
    async fn record_rejected(
        &self,
        payload: &[u8],
        signature: Option<&str>,
        reason: &str,
    ) -> Result<(), WebhookStoreError> {
        sqlx::query(
            "INSERT INTO payments.webhook_events \
             (event_id, payload, signature, accepted, reason) \
             VALUES (NULL, $1, $2, false, $3)",
        )
        .bind(payload)
        .bind(signature)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn record_and_claim(
        &self,
        event_id: &str,
        payload: &[u8],
    ) -> Result<bool, WebhookStoreError> {
        // Insert the accepted receipt; the partial unique index on event_id
        // makes a redelivery conflict, and `DO NOTHING RETURNING` yields no row
        // — so a returned row means "first time, claim it" and no row means
        // "already seen, duplicate".
        let row = sqlx::query(
            "INSERT INTO payments.webhook_events (event_id, payload, accepted) \
             VALUES ($1, $2, true) \
             ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING \
             RETURNING id",
        )
        .bind(event_id)
        .bind(payload)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        Ok(row.is_some())
    }
}

impl PgWebhookEventStore {
    /// Count the receipts stored for `event_id` — a test/diagnostics helper for
    /// confirming a duplicate was still logged exactly once as accepted.
    pub async fn accepted_count(&self, event_id: &str) -> Result<i64, WebhookStoreError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM payments.webhook_events \
             WHERE event_id = $1 AND accepted",
        )
        .bind(event_id)
        .fetch_one(&self.pool)
        .await
        .map_err(backend)?;
        row.try_get("n").map_err(backend)
    }
}
