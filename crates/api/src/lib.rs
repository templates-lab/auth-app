//! API layer — the HTTP boundary.
//!
//! Depends on `application` (and `domain` for its types) and on `axum`. It
//! translates HTTP requests into application calls and back; it holds no
//! business logic and no storage concerns.

use application::HealthService;
use axum::{extract::State, http::StatusCode, routing::get, Router};
use domain::Readiness;

/// Build the HTTP router, injecting the application services as state.
///
/// The router is intentionally minimal — a single readiness probe — so the
/// composition root can boot an HTTP server before any feature endpoints exist.
/// New modules add their routes here as they land.
pub fn router(health: HealthService) -> Router {
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
