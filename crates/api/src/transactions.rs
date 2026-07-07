//! HTTP boundary for the admin transactions view (bead authapp-a18fa6): a
//! filtered, paginated payment list, a single payment's detail with its full
//! status history, and an admin-only refund action.
//!
//! Holds no business logic — [`PaymentsService`] owns listing, history, and the
//! refund state machine; this module only translates HTTP to/from it. The two
//! read endpoints require any authenticated session; the refund additionally
//! requires the `admin` role, enforced by [`require_role`] (AC: reembolso solo
//! para admin).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use application::{PaymentsService, RefundError};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::Role;
use payments::{
    Payment, PaymentId, PaymentQuery, PaymentStatus, PaymentStatusChange, ProviderError,
};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ErrorResponse};
use crate::rbac::require_role;
use crate::session::require_session;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// Mount the transactions routes.
///
/// The list/detail reads are gated by [`require_session`] alone (any
/// authenticated admin may view). The refund mutation is gated additionally by
/// [`require_role`] for the `admin` role — the same layer ordering as
/// [`crate::audit`]: `require_role` is added first (inner, runs second) so it
/// reads the [`crate::session::CurrentSession`] the outer `require_session`
/// populated. As a mutation, refund also carries CSRF enforcement via
/// `require_session`.
pub fn routes(payments: PaymentsService, sessions: application::SessionService) -> Router {
    let read = Router::new()
        .route("/transactions", get(list_transactions))
        .route("/transactions/{id}", get(get_transaction))
        .with_state(payments.clone())
        .layer(axum::middleware::from_fn_with_state(
            sessions.clone(),
            require_session,
        ));

    let refund = Router::new()
        .route("/transactions/{id}/refund", post(refund_transaction))
        .with_state(payments)
        .layer(axum::middleware::from_fn_with_state(
            Role::admin(),
            require_role,
        ))
        .layer(axum::middleware::from_fn_with_state(
            sessions,
            require_session,
        ));

    read.merge(refund)
}

/// `?status=&created_after=&created_before=&limit=&offset=` — every field
/// optional. Dates are Unix epoch seconds, matching the timestamps the payloads
/// return.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListQuery {
    /// Only payments in this status (e.g. `captured`, `refunded`).
    status: Option<String>,
    /// Only payments created at or after this Unix epoch second (inclusive).
    created_after: Option<i64>,
    /// Only payments created strictly before this Unix epoch second.
    created_before: Option<i64>,
    /// Page size (default 50, capped at 200).
    limit: Option<u32>,
    /// Rows to skip, for paging (default 0).
    offset: Option<u32>,
}

/// One payment row, as returned to the admin panel.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TransactionOut {
    /// The payment's opaque id (its primary key).
    id: String,
    /// The provider's reference, once one exists.
    provider_reference: Option<String>,
    /// The amount, in the currency's minor units (e.g. cents) — never a float.
    amount_minor_units: i64,
    /// ISO 4217 currency code (`USD`, ...).
    currency: String,
    /// The current status (`created`, `captured`, `refunded`, ...).
    status: String,
    /// When the payment was created, as Unix epoch seconds.
    created_at_epoch: u64,
    /// When the payment's status last changed, as Unix epoch seconds.
    updated_at_epoch: u64,
}

impl From<Payment> for TransactionOut {
    fn from(p: Payment) -> Self {
        Self {
            id: p.id.as_str().to_string(),
            provider_reference: p.provider_reference.map(|r| r.as_str().to_string()),
            amount_minor_units: p.amount.minor_units(),
            currency: p.amount.currency().as_str().to_string(),
            status: p.status.as_str().to_string(),
            created_at_epoch: to_epoch(p.created_at),
            updated_at_epoch: to_epoch(p.updated_at),
        }
    }
}

/// One page of transactions plus the total matching the filter.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TransactionPage {
    /// The payments on this page, newest first.
    items: Vec<TransactionOut>,
    /// Total payments matching the filter across all pages.
    total: u64,
    /// The page size that was applied (after capping).
    limit: u32,
    /// The offset that was applied.
    offset: u32,
}

/// One recorded status change in a payment's history.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StatusChangeOut {
    /// The prior status, or `null` for the row recording creation.
    from: Option<String>,
    /// The status moved to.
    to: String,
    /// An optional human-readable reason (a decline code, `admin_refund`, ...).
    reason: Option<String>,
    /// When the transition happened, as Unix epoch seconds.
    occurred_at_epoch: u64,
}

impl From<PaymentStatusChange> for StatusChangeOut {
    fn from(c: PaymentStatusChange) -> Self {
        Self {
            from: c.from.map(|s| s.as_str().to_string()),
            to: c.to.as_str().to_string(),
            reason: c.reason,
            occurred_at_epoch: to_epoch(c.occurred_at),
        }
    }
}

/// A payment with its full status history — the detail view's payload.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TransactionDetailOut {
    /// The payment itself.
    transaction: TransactionOut,
    /// Every recorded status change, oldest first.
    history: Vec<StatusChangeOut>,
}

/// The result of a successful refund.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RefundOut {
    /// The status the payment moved to (`refunded` or `partially_refunded`).
    status: String,
}

/// Handle `GET /transactions`: a filtered, paginated payment list, newest
/// first.
#[utoipa::path(
    get,
    path = "/transactions",
    params(ListQuery),
    responses(
        (status = 200, description = "One page of transactions", body = TransactionPage),
        (status = 401, description = "No valid session", body = ErrorResponse),
        (status = 422, description = "Invalid status filter (per-field)", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
    tag = "transactions",
)]
pub(crate) async fn list_transactions(
    State(payments): State<PaymentsService>,
    Query(q): Query<ListQuery>,
) -> Result<Json<TransactionPage>, ApiError> {
    let status = match q.status.as_deref().map(PaymentStatus::parse).transpose() {
        Ok(status) => status,
        Err(_) => {
            let mut fields = BTreeMap::new();
            fields.insert("status".to_string(), "unknown payment status".to_string());
            return Err(ApiError::validation(fields));
        }
    };
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = q.offset.unwrap_or(0);

    let query = PaymentQuery {
        status,
        created_after: q.created_after.map(from_epoch),
        created_before: q.created_before.map(from_epoch),
        limit,
        offset,
    };

    let page = payments
        .list(&query)
        .await
        .map_err(|e| ApiError::internal(format!("transactions: list failed: {e}")))?;
    Ok(Json(TransactionPage {
        items: page.items.into_iter().map(TransactionOut::from).collect(),
        total: page.total,
        limit,
        offset,
    }))
}

/// Handle `GET /transactions/{id}`: one payment with its full status history.
#[utoipa::path(
    get,
    path = "/transactions/{id}",
    params(("id" = String, Path, description = "The payment id")),
    responses(
        (status = 200, description = "The payment and its status history", body = TransactionDetailOut),
        (status = 401, description = "No valid session", body = ErrorResponse),
        (status = 404, description = "No such payment", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
    tag = "transactions",
)]
pub(crate) async fn get_transaction(
    State(payments): State<PaymentsService>,
    Path(id): Path<String>,
) -> Result<Json<TransactionDetailOut>, ApiError> {
    let detail = payments
        .get(&PaymentId::new(id))
        .await
        .map_err(|e| ApiError::internal(format!("transactions: get failed: {e}")))?
        .ok_or_else(|| ApiError::not_found("No such transaction."))?;
    Ok(Json(TransactionDetailOut {
        transaction: TransactionOut::from(detail.payment),
        history: detail
            .history
            .into_iter()
            .map(StatusChangeOut::from)
            .collect(),
    }))
}

/// Handle `POST /transactions/{id}/refund`: refund a captured payment in full.
/// Admin-only (via [`require_role`]) and CSRF-checked (via [`require_session`]).
#[utoipa::path(
    post,
    path = "/transactions/{id}/refund",
    params(("id" = String, Path, description = "The payment id")),
    responses(
        (status = 200, description = "Refunded; the payment's new status", body = RefundOut),
        (status = 401, description = "No valid session", body = ErrorResponse),
        (status = 403, description = "Not the admin role, or missing/mismatched CSRF", body = ErrorResponse),
        (status = 404, description = "No such payment", body = ErrorResponse),
        (status = 409, description = "Payment is not in a refundable state", body = ErrorResponse),
        (status = 422, description = "The provider declined the refund", body = ErrorResponse),
        (status = 502, description = "The provider was unavailable", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
    tag = "transactions",
)]
pub(crate) async fn refund_transaction(
    State(payments): State<PaymentsService>,
    Path(id): Path<String>,
) -> Result<Json<RefundOut>, ApiError> {
    match payments.refund(&PaymentId::new(id)).await {
        Ok(outcome) => Ok(Json(RefundOut {
            status: outcome.status.as_str().to_string(),
        })),
        Err(RefundError::NotFound) => Err(ApiError::not_found("No such transaction.")),
        Err(
            RefundError::NotRefundable(_)
            | RefundError::NoProviderReference
            | RefundError::Conflict,
        ) => Err(ApiError::conflict(
            "not_refundable",
            "This payment is not in a refundable state.",
        )),
        Err(RefundError::Provider(ProviderError::Rejected(_))) => Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "provider_rejected",
            "The payment provider declined the refund.",
        )),
        Err(RefundError::Provider(ProviderError::Unavailable(_))) => Err(ApiError::bad_gateway(
            "provider_unavailable",
            "The payment provider is unavailable. Please try again.",
        )),
        Err(RefundError::Backend(msg)) => Err(ApiError::internal(format!(
            "transactions: refund failed: {msg}"
        ))),
    }
}

/// A `SystemTime` as whole Unix epoch seconds (saturating at the epoch for any
/// pre-1970 instant, which never occurs for stored timestamps).
fn to_epoch(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A Unix epoch second count as a `SystemTime` (negative values, which the API
/// never produces, clamp to the epoch).
fn from_epoch(seconds: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds.max(0) as u64)
}
