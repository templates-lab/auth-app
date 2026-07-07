//! Postgres adapter for the [`PaymentRepository`] port, over the `payments`
//! schema (`payments.payments` / `payments.payment_status_history`).
//!
//! [`PgPaymentRepository::transition`] is the one method that ever changes a
//! stored status: it updates the row and appends a history entry inside a
//! single transaction, guarded by a `WHERE status = $expected_current` clause
//! so two concurrent transitions cannot both win — the loser sees
//! [`PaymentRepositoryError::Conflict`].
//!
//! Timestamps cross the boundary as Unix epoch seconds, the same convention
//! [`crate::admin_repo`] uses.

use std::time::SystemTime;

use async_trait::async_trait;
use payments::{
    Currency, Money, NewPayment, Payment, PaymentId, PaymentRepository, PaymentRepositoryError,
    PaymentStatus, PaymentStatusChange, ProviderReference,
};
use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::admin_repo::{from_epoch, to_epoch};

/// Map an arbitrary sqlx error to a backend [`PaymentRepositoryError`].
fn backend(err: sqlx::Error) -> PaymentRepositoryError {
    PaymentRepositoryError::Backend(err.to_string())
}

/// Map a data-integrity failure (a stored value that no longer parses) to a
/// backend error — the write path only ever stores values these parsers
/// accept, so a failure here means the schema and the Rust types have
/// drifted.
fn corrupt(context: &str, err: impl std::fmt::Display) -> PaymentRepositoryError {
    PaymentRepositoryError::Backend(format!("{context}: {err}"))
}

/// A Postgres-backed [`PaymentRepository`].
#[derive(Debug, Clone)]
pub struct PgPaymentRepository {
    pool: PgPool,
}

impl PgPaymentRepository {
    /// Build the repository over an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PaymentRepository for PgPaymentRepository {
    async fn insert(&self, payment: &NewPayment) -> Result<PaymentId, PaymentRepositoryError> {
        let row = sqlx::query(
            "INSERT INTO payments.payments \
             (amount_minor_units, currency, status, created_at, updated_at) \
             VALUES ($1, $2, $3, to_timestamp($4), to_timestamp($4)) \
             RETURNING id::text AS id",
        )
        .bind(payment.amount.minor_units())
        .bind(payment.amount.currency().as_str())
        .bind(PaymentStatus::Created.as_str())
        .bind(to_epoch(payment.created_at))
        .fetch_one(&self.pool)
        .await
        .map_err(backend)?;

        let id: String = row.try_get("id").map_err(backend)?;

        // The history row for creation itself: no prior status.
        sqlx::query(
            "INSERT INTO payments.payment_status_history \
             (payment_id, from_status, to_status, reason, occurred_at) \
             VALUES ($1::uuid, NULL, $2, NULL, to_timestamp($3))",
        )
        .bind(&id)
        .bind(PaymentStatus::Created.as_str())
        .bind(to_epoch(payment.created_at))
        .execute(&self.pool)
        .await
        .map_err(backend)?;

        Ok(PaymentId::new(id))
    }

    async fn find(&self, id: &PaymentId) -> Result<Option<Payment>, PaymentRepositoryError> {
        let row = sqlx::query(
            "SELECT id::text AS id, provider_reference, amount_minor_units, currency, status, \
             EXTRACT(EPOCH FROM created_at)::bigint AS created_at_epoch, \
             EXTRACT(EPOCH FROM updated_at)::bigint AS updated_at_epoch \
             FROM payments.payments WHERE id = $1::uuid",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(row_to_payment(row)?))
    }

    async fn find_by_provider_reference(
        &self,
        reference: &ProviderReference,
    ) -> Result<Option<Payment>, PaymentRepositoryError> {
        let row = sqlx::query(
            "SELECT id::text AS id, provider_reference, amount_minor_units, currency, status, \
             EXTRACT(EPOCH FROM created_at)::bigint AS created_at_epoch, \
             EXTRACT(EPOCH FROM updated_at)::bigint AS updated_at_epoch \
             FROM payments.payments WHERE provider_reference = $1",
        )
        .bind(reference.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(row_to_payment(row)?))
    }

    async fn set_provider_reference(
        &self,
        id: &PaymentId,
        reference: &ProviderReference,
    ) -> Result<(), PaymentRepositoryError> {
        sqlx::query("UPDATE payments.payments SET provider_reference = $2 WHERE id = $1::uuid")
            .bind(id.as_str())
            .bind(reference.as_str())
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn transition(
        &self,
        id: &PaymentId,
        expected_current: PaymentStatus,
        next: PaymentStatus,
        reason: Option<&str>,
        occurred_at: SystemTime,
    ) -> Result<(), PaymentRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        let result = sqlx::query(
            "UPDATE payments.payments SET status = $3, updated_at = to_timestamp($4) \
             WHERE id = $1::uuid AND status = $2",
        )
        .bind(id.as_str())
        .bind(expected_current.as_str())
        .bind(next.as_str())
        .bind(to_epoch(occurred_at))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        if result.rows_affected() == 0 {
            // Rolling back is implicit on drop, but explicit here documents
            // the intent: no history row without the matching state change.
            tx.rollback().await.map_err(backend)?;
            return Err(PaymentRepositoryError::Conflict);
        }

        sqlx::query(
            "INSERT INTO payments.payment_status_history \
             (payment_id, from_status, to_status, reason, occurred_at) \
             VALUES ($1::uuid, $2, $3, $4, to_timestamp($5))",
        )
        .bind(id.as_str())
        .bind(expected_current.as_str())
        .bind(next.as_str())
        .bind(reason)
        .bind(to_epoch(occurred_at))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn history(
        &self,
        id: &PaymentId,
    ) -> Result<Vec<PaymentStatusChange>, PaymentRepositoryError> {
        let rows = sqlx::query(
            "SELECT from_status, to_status, reason, \
             EXTRACT(EPOCH FROM occurred_at)::bigint AS occurred_at_epoch \
             FROM payments.payment_status_history \
             WHERE payment_id = $1::uuid ORDER BY id ASC",
        )
        .bind(id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.into_iter()
            .map(|row| {
                let from_status: Option<String> = row.try_get("from_status").map_err(backend)?;
                let to_status: String = row.try_get("to_status").map_err(backend)?;
                let reason: Option<String> = row.try_get("reason").map_err(backend)?;
                let occurred_at: i64 = row.try_get("occurred_at_epoch").map_err(backend)?;

                Ok(PaymentStatusChange {
                    payment_id: id.clone(),
                    from: from_status
                        .map(|s| PaymentStatus::parse(&s))
                        .transpose()
                        .map_err(|e| corrupt("stored from_status", e))?,
                    to: PaymentStatus::parse(&to_status)
                        .map_err(|e| corrupt("stored to_status", e))?,
                    reason,
                    occurred_at: from_epoch(occurred_at),
                })
            })
            .collect()
    }
}

fn row_to_payment(row: sqlx::postgres::PgRow) -> Result<Payment, PaymentRepositoryError> {
    let id: String = row.try_get("id").map_err(backend)?;
    let provider_reference: Option<String> = row.try_get("provider_reference").map_err(backend)?;
    let amount_minor_units: i64 = row.try_get("amount_minor_units").map_err(backend)?;
    let currency: String = row.try_get("currency").map_err(backend)?;
    let status: String = row.try_get("status").map_err(backend)?;
    let created_at: i64 = row.try_get("created_at_epoch").map_err(backend)?;
    let updated_at: i64 = row.try_get("updated_at_epoch").map_err(backend)?;

    let currency = Currency::parse(&currency).map_err(|e| corrupt("stored currency", e))?;
    let amount = Money::from_minor_units(amount_minor_units, currency)
        .map_err(|e| corrupt("stored amount", e))?;
    let status = PaymentStatus::parse(&status).map_err(|e| corrupt("stored status", e))?;

    Ok(Payment {
        id: PaymentId::new(id),
        provider_reference: provider_reference.map(ProviderReference::new),
        amount,
        status,
        created_at: from_epoch(created_at),
        updated_at: from_epoch(updated_at),
    })
}
