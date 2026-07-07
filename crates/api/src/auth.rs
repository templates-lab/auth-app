//! HTTP boundary for administrator login.
//!
//! Translates `POST /auth/login` into a call on the application
//! [`LoginService`] and maps the outcome back to a status code. It holds no
//! business logic: every security decision (constant-time verification,
//! lockout) lives in the application and domain layers; this module only speaks
//! HTTP.

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
use domain::{AuditEventType, NewAuditEvent};
use serde::{Deserialize, Serialize};

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::session::attach_session_cookies;

/// State for the login route: the credential check, the session issuer a
/// successful login hands off to, the audit trail, and the app-level rate
/// limiter guarding the route independently of Traefik's edge-wide limit.
#[derive(Clone)]
struct LoginState {
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
#[derive(Debug, Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

/// The success body: the authenticated administrator's id.
#[derive(Debug, Serialize)]
struct LoginOk {
    admin_id: String,
}

/// A uniform error body. The `error` code is deliberately coarse so it never
/// distinguishes "no such account" from "wrong password".
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
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
async fn login_handler(
    State(state): State<LoginState>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Response {
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
            eprintln!(
                "login: rate limit exceeded for {key}, retry after {}s",
                exceeded.retry_after.as_secs()
            );
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorBody {
                    error: "too_many_attempts",
                }),
            )
                .into_response();
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
        Ok(id) => {
            if let Err(e) = audit(AuditEventType::LoginSucceeded, Some(id.clone())).await {
                eprintln!("login: failed to record audit event: {e}");
            }
            match state.sessions.start(id.clone()).await {
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
                Err(e) => {
                    eprintln!("login: failed to issue session: {e}");
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
        Err(LoginError::InvalidCredentials) => {
            if let Err(e) = audit(AuditEventType::LoginFailed, None).await {
                eprintln!("login: failed to record audit event: {e}");
            }
            (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody {
                    error: "invalid_credentials",
                }),
            )
                .into_response()
        }
        Err(LoginError::TooManyAttempts { retry_after_secs }) => {
            if let Err(e) = audit(AuditEventType::LockedOut, None).await {
                eprintln!("login: failed to record audit event: {e}");
            }
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorBody {
                    error: "too_many_attempts",
                }),
            )
                .into_response();
            if let Some(secs) = retry_after_secs {
                if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
                    response.headers_mut().insert(header::RETRY_AFTER, value);
                }
            }
            response
        }
        // Details are logged server-side; the client only learns it was our fault.
        Err(LoginError::Internal(msg)) => {
            eprintln!("login: internal error: {msg}");
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
