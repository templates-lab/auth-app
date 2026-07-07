//! HTTP boundary for inbound payment webhooks: a public endpoint the provider
//! POSTs to. It reads the *raw* body (the signature is computed over these
//! exact bytes) and the provider signature header, hands both to
//! [`WebhookService`], and maps the outcome to a status code.
//!
//! No session or CSRF layer — the caller is the payment provider, authenticated
//! by the webhook signature, not by a cookie.

use application::{WebhookOutcome, WebhookService};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;

/// The header carrying the provider's signature (Stripe's name; a different
/// provider adapter would read its own, but the route shape is the same).
pub const SIGNATURE_HEADER: &str = "stripe-signature";

/// Mount the webhook route.
pub fn routes(webhooks: WebhookService) -> Router {
    Router::new()
        .route("/payments/webhooks", post(handle))
        .with_state(webhooks)
}

/// Handle `POST /payments/webhooks`:
///
/// - `200` — accepted, a no-op we recognize, or a deduplicated redelivery
///   (all of which the provider should treat as success and not retry).
/// - `400` — bad/missing signature or malformed payload (recorded for audit).
/// - `500` — an internal failure, so the provider retries later.
async fn handle(State(svc): State<WebhookService>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let signature = headers.get(SIGNATURE_HEADER).and_then(|v| v.to_str().ok());
    match svc.handle(&body, signature).await {
        WebhookOutcome::Processed | WebhookOutcome::Ignored | WebhookOutcome::Duplicate => {
            StatusCode::OK
        }
        WebhookOutcome::Rejected => StatusCode::BAD_REQUEST,
        WebhookOutcome::Error(msg) => {
            tracing::error!("webhook: internal error: {msg}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
