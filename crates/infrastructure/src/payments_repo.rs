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
    Currency, Money, NewPayment, Payment, PaymentId, PaymentQuery, PaymentRepository,
    PaymentRepositoryError, PaymentStatus, PaymentStatusChange, ProviderReference,
};
use sqlx::postgres::{PgArguments, PgPool};
use sqlx::{Arguments, Row};

use crate::admin_repo::{from_epoch, to_epoch};

/// The hard cap on how many payments one [`PaymentRepository::list`] call
/// returns, regardless of the requested `limit` — the same defence against an
/// unbounded scan the audit endpoint applies.
const MAX_LIST_LIMIT: u32 = 200;

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

    async fn list(&self, query: &PaymentQuery) -> Result<Vec<Payment>, PaymentRepositoryError> {
        // Build the shared WHERE fragment, then append paging. Placeholders are
        // numbered as filters are added, so `$next` always matches the next
        // bound argument regardless of which filters are present.
        let mut filter = FilterSql::new();
        filter.push_conditions(query);
        let limit = query.limit.clamp(1, MAX_LIST_LIMIT);

        let limit_placeholder = filter.next_placeholder();
        filter.args.add(limit as i64).map_err(bind)?;
        let offset_placeholder = filter.next_placeholder();
        filter.args.add(query.offset as i64).map_err(bind)?;

        let sql = format!(
            "SELECT id::text AS id, provider_reference, amount_minor_units, currency, status, \
             EXTRACT(EPOCH FROM created_at)::bigint AS created_at_epoch, \
             EXTRACT(EPOCH FROM updated_at)::bigint AS updated_at_epoch \
             FROM payments.payments{} \
             ORDER BY created_at DESC, id DESC LIMIT {limit_placeholder} OFFSET {offset_placeholder}",
            filter.where_clause(),
        );

        let rows = sqlx::query_with(&sql, filter.args)
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;

        rows.into_iter().map(row_to_payment).collect()
    }

    async fn count(&self, query: &PaymentQuery) -> Result<u64, PaymentRepositoryError> {
        let mut filter = FilterSql::new();
        filter.push_conditions(query);

        let sql = format!(
            "SELECT COUNT(*) AS total FROM payments.payments{}",
            filter.where_clause(),
        );

        let row = sqlx::query_with(&sql, filter.args)
            .fetch_one(&self.pool)
            .await
            .map_err(backend)?;
        let total: i64 = row.try_get("total").map_err(backend)?;
        Ok(total.max(0) as u64)
    }
}

/// Map a query-argument binding failure to a backend error.
fn bind(err: sqlx::error::BoxDynError) -> PaymentRepositoryError {
    PaymentRepositoryError::Backend(format!("binding query argument: {err}"))
}

/// Accumulates the optional `WHERE` conditions of a [`PaymentQuery`] and their
/// bound arguments, keeping placeholder numbers and argument order in lockstep
/// so `list` and `count` share one definition of "which rows match".
struct FilterSql {
    conditions: Vec<String>,
    args: PgArguments,
}

impl FilterSql {
    fn new() -> Self {
        Self {
            conditions: Vec::new(),
            args: PgArguments::default(),
        }
    }

    /// The 1-based placeholder for the argument that will be added next.
    fn next_placeholder(&self) -> String {
        format!("${}", self.args.len() + 1)
    }

    /// Add the status/date filters present in `query`, binding one argument per
    /// condition.
    fn push_conditions(&mut self, query: &PaymentQuery) {
        if let Some(status) = query.status {
            let p = self.next_placeholder();
            // `status.as_str()` is a &'static str; bind an owned copy.
            self.args
                .add(status.as_str().to_string())
                .expect("binding a String never fails");
            self.conditions.push(format!("status = {p}"));
        }
        if let Some(after) = query.created_after {
            let p = self.next_placeholder();
            self.args
                .add(to_epoch(after))
                .expect("binding an i64 never fails");
            self.conditions
                .push(format!("created_at >= to_timestamp({p})"));
        }
        if let Some(before) = query.created_before {
            let p = self.next_placeholder();
            self.args
                .add(to_epoch(before))
                .expect("binding an i64 never fails");
            self.conditions
                .push(format!("created_at < to_timestamp({p})"));
        }
    }

    /// The ` WHERE a AND b` clause (with a leading space), or an empty string
    /// when no filter is present.
    fn where_clause(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        }
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
