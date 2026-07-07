//! Minimal RBAC (bead authapp-e00d47): a middleware that rejects a request
//! with `403` unless the authenticated session's role matches the one an
//! endpoint requires.
//!
//! Apply this **after** [`crate::session::require_session`] in the layer
//! chain — concretely, call `.layer()` for this middleware *before* calling
//! `.layer()` for `require_session`, since the later `.layer()` call becomes
//! the outer, first-executed wrapper (axum/tower run the last-added layer
//! first). `require_role` reads the [`CurrentSession`] `require_session`
//! populates; it does not authenticate the session itself.
//!
//! Adding a new role never touches this middleware — [`Role`] is a validated
//! string, not a closed set, so gating a new endpoint to a new role is a
//! one-line `require_role(Role::parse("editor")?)`, not a structural change.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::Role;
use serde::Serialize;

use crate::session::CurrentSession;

/// Require that the authenticated session's role equals `required`, exactly.
///
/// Rejects with `403` if [`CurrentSession`] is missing (i.e. `require_session`
/// did not run first — a router-wiring bug, not a client error, but `403` is
/// the safe default over a `500`) or its role does not match.
pub async fn require_role(State(required): State<Role>, request: Request, next: Next) -> Response {
    let matches = request
        .extensions()
        .get::<CurrentSession>()
        .is_some_and(|current| current.role == required);

    if !matches {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorBody { error: "forbidden" }),
        )
            .into_response();
    }

    next.run(request).await
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}
