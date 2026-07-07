//! HTTP boundary for OAuth sign-in: the redirect that starts a flow and the
//! callback that finishes it, issuing a session exactly like a password login.
//!
//! Holds no business logic — [`OAuthLoginService`] owns the flow; this module
//! only speaks HTTP (redirects, cookies). Both routes are public (no session
//! exists yet), so they carry no `require_session` layer.

use application::{AuditService, OAuthLoginService, SessionService};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use domain::{AdminId, AuditEventType, NewAuditEvent};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::auth::ClientIp;
use crate::error::ApiError;
use crate::session::attach_session_cookies;

/// Where the callback sends the browser after it finishes.
#[derive(Debug, Clone)]
pub struct OAuthRedirects {
    /// Path to redirect to after a successful sign-in (default `/`).
    pub success: String,
    /// Path to redirect to after a failed sign-in; an `?error=oauth` query is
    /// appended (default `/login`).
    pub failure: String,
}

impl Default for OAuthRedirects {
    fn default() -> Self {
        Self {
            success: "/".to_string(),
            failure: "/login".to_string(),
        }
    }
}

/// State shared by the OAuth routes.
#[derive(Clone)]
pub(crate) struct OAuthState {
    oauth: OAuthLoginService,
    sessions: SessionService,
    audit: AuditService,
    redirects: OAuthRedirects,
}

/// Mount the OAuth routes.
pub fn routes(
    oauth: OAuthLoginService,
    sessions: SessionService,
    audit: AuditService,
    redirects: OAuthRedirects,
) -> Router {
    Router::new()
        .route("/auth/oauth/providers", get(providers_handler))
        .route("/auth/oauth/{provider}/start", get(start_handler))
        .route("/auth/oauth/{provider}/callback", get(callback_handler))
        .with_state(OAuthState {
            oauth,
            sessions,
            audit,
            redirects,
        })
}

/// The configured OAuth providers, for the login UI to render a button per one.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProvidersOut {
    /// The enabled provider ids (e.g. `["google"]`), sorted. Empty when OAuth
    /// is configured but has no providers; the whole route is absent (`404`)
    /// when OAuth is disabled entirely — the UI treats both as "no providers".
    providers: Vec<String>,
}

/// `GET /auth/oauth/providers`: list the enabled provider ids. Public — the
/// login page calls it before any session exists.
#[utoipa::path(
    get,
    path = "/auth/oauth/providers",
    responses(
        (status = 200, description = "The enabled OAuth provider ids", body = ProvidersOut),
    ),
    tag = "auth",
)]
pub(crate) async fn providers_handler(State(state): State<OAuthState>) -> Response {
    Json(ProvidersOut {
        providers: state.oauth.provider_ids(),
    })
    .into_response()
}

/// `GET /auth/oauth/{provider}/start`: redirect the browser to the provider's
/// authorize URL. `404` for an unknown provider; `500` if the flow cannot be
/// started (a storage failure).
async fn start_handler(Path(provider): Path<String>, State(state): State<OAuthState>) -> Response {
    if !state.oauth.has_provider(&provider) {
        return ApiError::not_found("Unknown sign-in provider.").into_response();
    }
    match state.oauth.begin(&provider).await {
        Ok(outcome) => Redirect::to(&outcome.authorize_url).into_response(),
        Err(e) => ApiError::internal(format!("oauth: failed to begin flow for {provider}: {e}"))
            .into_response(),
    }
}

/// The callback query: `state` + `code` on success, or `error` if the provider
/// declined. All optional so a malformed callback is handled, not rejected by
/// extraction.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

/// `GET /auth/oauth/{provider}/callback`: finish the flow. On success issue a
/// session (fresh cookies, exactly like a password login), record the login to
/// the audit trail, and redirect to the success path. On any failure redirect
/// to the failure path with `?error=oauth` — the browser is mid-navigation, so
/// a redirect (not a JSON error) is the right shape here.
async fn callback_handler(
    Path(_provider): Path<String>,
    Query(query): Query<CallbackQuery>,
    ClientIp(client_ip): ClientIp,
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    State(state): State<OAuthState>,
) -> Response {
    // The provider declined, or the callback is missing its parameters.
    if query.error.is_some() {
        return state.redirect_failure();
    }
    let (Some(cb_state), Some(code)) = (query.state, query.code) else {
        return state.redirect_failure();
    };

    let authenticated = match state.oauth.complete(&cb_state, &code).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("oauth: callback failed: {e}");
            return state.redirect_failure();
        }
    };

    let issued = match state
        .sessions
        .start(authenticated.id.clone(), authenticated.role)
        .await
    {
        Ok(issued) => issued,
        Err(e) => {
            eprintln!("oauth: failed to issue session: {e}");
            return state.redirect_failure();
        }
    };

    // Best-effort audit, same reasoning as the password login handler.
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if let Err(e) = state
        .audit
        .record(NewAuditEvent {
            event_type: AuditEventType::LoginSucceeded,
            admin_id: Some(AdminId::new(authenticated.id.as_str().to_string())),
            email_attempted: None,
            ip: client_ip,
            user_agent,
            occurred_at: SystemTime::now(),
        })
        .await
    {
        eprintln!("oauth: failed to record audit event: {e}");
    }

    let jar = attach_session_cookies(jar, &issued);
    (jar, Redirect::to(&state.redirects.success)).into_response()
}

impl OAuthState {
    /// Redirect to the failure path with an `?error=oauth` marker.
    fn redirect_failure(&self) -> Response {
        let sep = if self.redirects.failure.contains('?') {
            '&'
        } else {
            '?'
        };
        Redirect::to(&format!("{}{sep}error=oauth", self.redirects.failure)).into_response()
    }
}
