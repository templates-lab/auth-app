//! HTTP boundary for administrator login.
//!
//! Translates `POST /auth/login` into a call on the application
//! [`LoginService`] and maps the outcome back to a status code. It holds no
//! business logic: every security decision (constant-time verification,
//! lockout) lives in the application and domain layers; this module only speaks
//! HTTP.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use application::{AuditService, LoginError, LoginRequest, LoginService, SessionService};
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use domain::{AuditEventType, Email, NewAuditEvent};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ErrorResponse};

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::session::attach_session_cookies;

/// State for the login route: the credential check, the session issuer a
/// successful login hands off to, the audit trail, and the app-level rate
/// limiter guarding the route independently of Traefik's edge-wide limit.
#[derive(Clone)]
pub(crate) struct LoginState {
    login: LoginService,
    sessions: SessionService,
    audit: AuditService,
    rate_limit: Arc<RateLimiter>,
}

/// Mount the auth routes with the login, session, and audit services as
/// their state.
///
/// `rate_limit` caps login attempts per client IP *and*, independently, per
/// submitted account email — Traefik's edge limit sees neither the parsed
/// body nor per-route semantics, so per-account limiting has to live here.
pub fn routes(
    login: LoginService,
    sessions: SessionService,
    audit: AuditService,
    rate_limit: RateLimitConfig,
) -> Router {
    Router::new()
        .route("/auth/login", post(login_handler))
        .with_state(LoginState {
            login,
            sessions,
            audit,
            rate_limit: Arc::new(RateLimiter::new(rate_limit)),
        })
}

/// The JSON body of a login request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginBody {
    /// The administrator's email address.
    #[schema(example = "admin@example.com")]
    email: String,
    /// The administrator's password.
    password: String,
}

/// The success body: the authenticated administrator's id.
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginOk {
    /// The authenticated administrator's opaque id.
    admin_id: String,
}

/// The client's IP address, resolved from proxy headers or the socket.
///
/// Behind Traefik the real client address arrives in `X-Forwarded-For` (the
/// first, client-most hop) or `X-Real-IP`; direct connections fall back to the
/// peer socket recorded in [`ConnectInfo`]. This is the identity the application
/// throttles per-IP, so getting it from the forwarded header — not the proxy's
/// own socket — is what makes IP lockout meaningful in production.
///
/// `pub(crate)` so [`crate::session`]'s logout handler can reuse the same
/// resolution logic for its own audit-event IP, rather than duplicating it.
pub(crate) struct ClientIp(pub(crate) String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(ip) = forwarded_ip(parts) {
            return Ok(Self(ip));
        }
        // `ConnectInfo` is present in the request extensions only when the server
        // is served with `into_make_service_with_connect_info`; absent that, we
        // degrade to a stable sentinel rather than failing the request.
        if let Some(ConnectInfo(addr)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() {
            return Ok(Self(addr.ip().to_string()));
        }
        Ok(Self("unknown".to_string()))
    }
}

/// Extract the client-most IP from `X-Forwarded-For`, else `X-Real-IP`.
fn forwarded_ip(parts: &Parts) -> Option<String> {
    let headers = &parts.headers;
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Handle `POST /auth/login`.
///
/// `200` with the admin id and fresh session/CSRF cookies on success; `401`
/// for invalid credentials (identical for a wrong password and a nonexistent
/// account); `429` with `Retry-After` when the IP or account has hit the
/// app-level rate limit, or when the account/IP is locked out (see
/// [`LoginError::TooManyAttempts`] — a distinct mechanism from the rate limit:
/// lockout counts *failures*, the rate limit counts *every* attempt); `500` on
/// an internal failure. Every successful login issues a brand-new session —
/// there is no "reuse the prior session" path — which is what satisfies
/// session rotation on login.
#[utoipa::path(
    post,
    path = "/auth/login",
    request_body = LoginBody,
    responses(
        (status = 200, description = "Signed in; sets session and csrf cookies", body = LoginOk),
        (status = 401, description = "Invalid credentials (wrong password or unknown account)", body = ErrorResponse),
        (status = 422, description = "Malformed request (per-field validation)", body = ErrorResponse),
        (status = 429, description = "Rate-limited or locked out; carries Retry-After", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse),
    ),
    tag = "auth",
)]
pub(crate) async fn login_handler(
    State(state): State<LoginState>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Response {
    // Validate the request shape first (AC: validation errors are 422 with
    // per-field detail). A malformed email or empty password is a request
    // problem, distinct from — and revealing nothing about — whether any account
    // exists, so surfacing the field is safe.
    let mut invalid = BTreeMap::new();
    if Email::parse(body.email.trim()).is_err() {
        invalid.insert(
            "email".to_string(),
            "must be a valid email address".to_string(),
        );
    }
    if body.password.is_empty() {
        invalid.insert("password".to_string(), "must not be empty".to_string());
    }
    if !invalid.is_empty() {
        return ApiError::validation(invalid).into_response();
    }

    let now = SystemTime::now();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let email_attempted = body.email.trim().to_ascii_lowercase();

    // Rate-limit by IP and by the submitted account independently, so an
    // attacker spraying many accounts from one IP is capped by the IP key,
    // and one hammering a single account from many IPs is capped by the
    // account key. Checked before any credential work — a rejected request
    // costs no argon2 verification and touches no lockout counters, and is
    // not itself an audited auth event (it never reached one).
    let account_key = format!("acct:{email_attempted}");
    for key in [format!("ip:{client_ip}"), account_key] {
        if let Err(exceeded) = state.rate_limit.check(&key, now) {
            tracing::warn!(
                key,
                retry_after_secs = exceeded.retry_after.as_secs(),
                "login rate limit exceeded"
            );
            let mut response = too_many_attempts().into_response();
            if let Ok(value) = HeaderValue::from_str(&exceeded.retry_after.as_secs().to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return response;
        }
    }

    let request = LoginRequest {
        email: body.email,
        password: body.password,
        client_ip: client_ip.clone(),
    };

    // Best-effort: an outage in the audit store must never block a real
    // login, so a `record` failure only logs — it does not change the
    // response.
    let audit = |event_type: AuditEventType, admin_id: Option<domain::AdminId>| {
        state.audit.record(NewAuditEvent {
            event_type,
            admin_id,
            email_attempted: Some(email_attempted.clone()),
            ip: client_ip.clone(),
            user_agent: user_agent.clone(),
            occurred_at: now,
        })
    };

    match state.login.login(request).await {
        Ok(authenticated) => {
            let id = authenticated.id;
            if let Err(e) = audit(AuditEventType::LoginSucceeded, Some(id.clone())).await {
                tracing::warn!("login: failed to record audit event: {e}");
            }
            match state.sessions.start(id.clone(), authenticated.role).await {
                Ok(issued) => {
                    let jar = attach_session_cookies(jar, &issued);
                    (
                        StatusCode::OK,
                        jar,
                        Json(LoginOk {
                            admin_id: id.as_str().to_string(),
                        }),
                    )
                        .into_response()
                }
                Err(e) => ApiError::internal(format!("login: failed to issue session: {e}"))
                    .into_response(),
            }
        }
        Err(LoginError::InvalidCredentials) => {
            if let Err(e) = audit(AuditEventType::LoginFailed, None).await {
                tracing::warn!("login: failed to record audit event: {e}");
            }
            // Coarse on purpose: identical for a wrong password and an unknown
            // account, so the response never reveals whether the email exists.
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid email or password.",
            )
            .into_response()
        }
        Err(LoginError::TooManyAttempts { retry_after_secs }) => {
            if let Err(e) = audit(AuditEventType::LockedOut, None).await {
                tracing::warn!("login: failed to record audit event: {e}");
            }
            let mut response = too_many_attempts().into_response();
            if let Some(secs) = retry_after_secs {
                if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
            }
            response
        }
        // Details are logged server-side; the client only learns it was our fault.
        Err(LoginError::Internal(msg)) => {
            ApiError::internal(format!("login: internal error: {msg}")).into_response()
        }
    }
}

/// The `429` returned both by the pre-check rate limiter and by a lockout. The
/// caller attaches the `Retry-After` header, since its value differs per source.
fn too_many_attempts() -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
        "Too many attempts. Please try again later.",
    )
}
