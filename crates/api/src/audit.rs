//! HTTP boundary for the audit trail: a read-only, admin-only endpoint the
//! admin panel's audit view (bead authapp-c418dc) queries.
//!
//! Holds no business logic — [`AuditService`] owns the query; this module
//! only translates it to/from HTTP.

use std::time::SystemTime;

use application::{AuditService, SessionService};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use domain::{AuditEvent, Role};
use serde::{Deserialize, Serialize};

use crate::rbac::require_role;
use crate::session::require_session;

/// Mount the audit routes, gated by [`require_session`] (any authenticated
/// session) *and* [`require_role`] (the `admin` role specifically) — viewing
/// the security audit trail is an admin-only action even in a scheme with
/// lower-privileged roles. `GET` is exempt from CSRF (it is not a mutation),
/// so no CSRF header is required here.
///
/// Layer order matters: `require_role` is added first (so it becomes the
/// inner layer, running second) and `require_session` last (the outer layer,
/// running first) — `require_role` reads the `CurrentSession` only
/// `require_session` has populated by the time it runs.
pub fn routes(audit: AuditService, sessions: SessionService) -> Router {
    Router::new()
        .route("/audit/events", get(list_events))
        .with_state(audit)
        .layer(axum::middleware::from_fn_with_state(
            Role::admin(),
            require_role,
        ))
        .layer(axum::middleware::from_fn_with_state(
            sessions,
            require_session,
        ))
}

/// `?limit=` query parameter, capped well below anything that could turn one
/// request into an unbounded table scan.
#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<u32>,
}

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// One audit event, as returned to the admin panel.
#[derive(Debug, Serialize)]
struct EventOut {
    id: String,
    event_type: String,
    admin_id: Option<String>,
    email_attempted: Option<String>,
    ip: String,
    user_agent: Option<String>,
    occurred_at_epoch: u64,
}

impl From<AuditEvent> for EventOut {
    fn from(e: AuditEvent) -> Self {
        Self {
            id: e.id.as_str().to_string(),
            event_type: e.event_type.as_str().to_string(),
            admin_id: e.admin_id.map(|id| id.as_str().to_string()),
            email_attempted: e.email_attempted,
            ip: e.ip,
            user_agent: e.user_agent,
            occurred_at_epoch: e
                .occurred_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

/// Handle `GET /audit/events?limit=`: the most recent audit events, newest
/// first, capped at [`MAX_LIMIT`] regardless of what the caller asks for.
async fn list_events(State(audit): State<AuditService>, Query(q): Query<ListQuery>) -> Response {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    match audit.recent(limit).await {
        Ok(events) => {
            Json(events.into_iter().map(EventOut::from).collect::<Vec<_>>()).into_response()
        }
        Err(e) => {
            eprintln!("audit: failed to list events: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "internal_error",
                }),
            )
                .into_response()
        }
    }
}
