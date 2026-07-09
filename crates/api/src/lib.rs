//! API layer — the HTTP boundary.
//!
//! Depends on `application` (and `domain` for its types) and on `axum`. It
//! translates HTTP requests into application calls and back; it holds no
//! business logic and no storage concerns.

use std::sync::Arc;

use application::{
    AuditService, HealthService, LoginService, OAuthLoginService, PaymentsService, SessionService,
    WebhookService,
};
use axum::{extract::State, http::StatusCode, routing::get, Router};
use domain::{AdminRepository, Readiness};

pub mod audit;
pub mod auth;
pub mod cors;
pub mod error;
pub mod oauth;
pub mod openapi;
pub mod payments_webhook;
pub mod rate_limit;
pub mod rbac;
pub mod session;
pub mod telemetry;
pub mod transactions;

use oauth::OAuthRedirects;
pub use openapi::ApiDoc;
use rate_limit::RateLimitConfig;

/// Build the HTTP router, injecting the application services as state.
///
/// Each concern is a self-contained sub-router carrying its own state, merged
/// onto a stateless base — so adding a delivery surface (here, admin login)
/// never entangles it with another's state. New features add a `.merge(...)`
/// line as they land. The CORS layer wraps the whole router last, so it
/// applies (and answers preflight `OPTIONS`) uniformly across every route.
///
/// `oauth`, `webhooks`, and `transactions` are optional: `None` simply leaves
/// those endpoints unmounted (OAuth returns `404`; the webhook and transactions
/// routes are absent). Transactions is mounted only when a payment provider is
/// configured, since a refund needs one.
// The composition root legitimately injects one argument per delivery surface;
// bundling them into a struct would only move the same wiring elsewhere.
#[allow(clippy::too_many_arguments)]
pub fn router(
    health: HealthService,
    login: LoginService,
    sessions: SessionService,
    audit: AuditService,
    admins: Arc<dyn AdminRepository>,
    oauth: Option<(OAuthLoginService, OAuthRedirects)>,
    webhooks: Option<WebhookService>,
    transactions: Option<PaymentsService>,
    cors_allowed_origins: &[String],
    login_rate_limit: RateLimitConfig,
) -> Router {
    let mut router = Router::new()
        .merge(health_routes(health))
        .merge(auth::routes(
            login,
            sessions.clone(),
            audit.clone(),
            login_rate_limit,
        ))
        .merge(session::routes(sessions.clone(), audit.clone(), admins))
        .merge(audit::routes(audit.clone(), sessions.clone()));
    if let Some((oauth, redirects)) = oauth {
        router = router.merge(oauth::routes(oauth, sessions.clone(), audit, redirects));
    }
    if let Some(webhooks) = webhooks {
        router = router.merge(payments_webhook::routes(webhooks));
    }
    if let Some(transactions) = transactions {
        router = router.merge(transactions::routes(transactions, sessions.clone()));
    }
    // CORS wraps every route; the request-context layer is added last so it is
    // the OUTERMOST wrapper — every request (CORS preflight included) runs inside
    // a `request` span with a trace id, echoed back in `x-request-id`.
    router
        .layer(cors::layer(cors_allowed_origins))
        .layer(axum::middleware::from_fn(telemetry::request_context))
}

/// The liveness + readiness sub-router.
///
/// `/health` is *liveness* — the process is up and answering, with no dependency
/// check, so it always returns `200` and never flaps a container restart on a
/// transient database blip. `/ready` is *readiness* — it probes the database, so
/// an orchestrator can hold traffic off an instance that cannot serve yet.
fn health_routes(health: HealthService) -> Router {
    Router::new()
        .route("/health", get(liveness_handler))
        .route("/ready", get(readiness_handler))
        .with_state(health)
}

/// Liveness probe: `200 OK` whenever the process is running. No dependency is
/// checked, so a slow or briefly-unreachable database never turns into a restart.
#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "The process is alive")),
    tag = "health",
)]
pub(crate) async fn liveness_handler() -> StatusCode {
    StatusCode::OK
}

/// Readiness probe: `200 OK` when ready, `503 Service Unavailable` otherwise.
///
/// Awaits the application service, which in turn probes the live database, so a
/// down or unreachable Postgres surfaces here as `503`.
#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "Ready to serve"),
        (status = 503, description = "Not ready (database unreachable)"),
    ),
    tag = "health",
)]
pub(crate) async fn readiness_handler(State(health): State<HealthService>) -> StatusCode {
    match health.health().await.readiness {
        Readiness::Ready => StatusCode::OK,
        Readiness::NotReady => StatusCode::SERVICE_UNAVAILABLE,
    }
}
