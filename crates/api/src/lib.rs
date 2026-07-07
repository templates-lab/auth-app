//! API layer — the HTTP boundary.
//!
//! Depends on `application` (and `domain` for its types) and on `axum`. It
//! translates HTTP requests into application calls and back; it holds no
//! business logic and no storage concerns.

use application::{
    AuditService, HealthService, LoginService, OAuthLoginService, SessionService, WebhookService,
};
use axum::{extract::State, http::StatusCode, routing::get, Router};
use domain::Readiness;

pub mod audit;
pub mod auth;
pub mod cors;
pub mod oauth;
pub mod payments_webhook;
pub mod rate_limit;
pub mod rbac;
pub mod session;

use oauth::OAuthRedirects;
use rate_limit::RateLimitConfig;

/// Build the HTTP router, injecting the application services as state.
///
/// Each concern is a self-contained sub-router carrying its own state, merged
/// onto a stateless base — so adding a delivery surface (here, admin login)
/// never entangles it with another's state. New features add a `.merge(...)`
/// line as they land. The CORS layer wraps the whole router last, so it
/// applies (and answers preflight `OPTIONS`) uniformly across every route.
///
/// `oauth` and `webhooks` are optional: `None` simply leaves those endpoints
/// unmounted (OAuth returns `404`; the webhook route is absent).
// The composition root legitimately injects one argument per delivery surface;
// bundling them into a struct would only move the same wiring elsewhere.
#[allow(clippy::too_many_arguments)]
pub fn router(
    health: HealthService,
    login: LoginService,
    sessions: SessionService,
    audit: AuditService,
    oauth: Option<(OAuthLoginService, OAuthRedirects)>,
    webhooks: Option<WebhookService>,
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
        .merge(session::routes(sessions.clone(), audit.clone()))
        .merge(audit::routes(audit.clone(), sessions.clone()));
    if let Some((oauth, redirects)) = oauth {
        router = router.merge(oauth::routes(oauth, sessions, audit, redirects));
    }
    if let Some(webhooks) = webhooks {
        router = router.merge(payments_webhook::routes(webhooks));
    }
    router.layer(cors::layer(cors_allowed_origins))
}

/// The readiness-probe sub-router.
fn health_routes(health: HealthService) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .with_state(health)
}

/// Readiness probe: `200 OK` when ready, `503 Service Unavailable` otherwise.
///
/// Awaits the application service, which in turn probes the live database, so a
/// down or unreachable Postgres surfaces here as `503`.
async fn health_handler(State(health): State<HealthService>) -> StatusCode {
    match health.health().await.readiness {
        Readiness::Ready => StatusCode::OK,
        Readiness::NotReady => StatusCode::SERVICE_UNAVAILABLE,
    }
}
