//! HTTP boundary for sessions: the cookies a login response sets, the
//! middleware that authenticates them (and checks CSRF on mutations) for every
//! protected route, and the `/auth/logout` endpoint that revokes one.
//!
//! Holds no business logic — every decision (session validity, expiry, CSRF
//! matching) lives in [`SessionService`]; this module only speaks HTTP and
//! cookies.

use std::time::SystemTime;

use application::{AuditService, IssuedSession, SessionError, SessionService};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use domain::{AdminId, AuditEventType, NewAuditEvent, Role, SessionToken};
use serde::Serialize;
use time::OffsetDateTime;

use crate::auth::ClientIp;

/// The `HttpOnly` cookie carrying the session bearer token.
pub const SESSION_COOKIE: &str = "session";
/// The client-readable cookie carrying the CSRF token, mirrored back by the
/// caller in [`CSRF_HEADER`] on every mutation.
pub const CSRF_COOKIE: &str = "csrf";
/// The request header a mutating call must echo the CSRF cookie's value into.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// State for the logout route: the session service it revokes through, and
/// the audit trail it records the logout to.
#[derive(Clone)]
pub(crate) struct LogoutState {
    sessions: SessionService,
    audit: AuditService,
}

/// Mount the routes that require an authenticated session: `/auth/me` (any
/// role) and `/auth/logout`. Future protected mutations mount alongside these
/// the same way.
pub fn routes(sessions: SessionService, audit: AuditService) -> Router {
    Router::new()
        .route("/auth/logout", post(logout_handler))
        .route("/auth/me", get(me_handler))
        .with_state(LogoutState {
            sessions: sessions.clone(),
            audit,
        })
        .layer(axum::middleware::from_fn_with_state(
            sessions,
            require_session,
        ))
}

/// The authenticated identity a protected handler can extract from request
/// extensions, populated by [`require_session`].
#[derive(Debug, Clone)]
pub struct CurrentSession {
    /// The authenticated administrator's id.
    pub admin_id: String,
    /// The authenticated administrator's role — the same value `/auth/me`
    /// reports and [`crate::rbac::require_role`] checks.
    pub role: Role,
    /// The raw session token, so a handler that ends the session (logout,
    /// rotation) can act on the exact row without a second cookie read.
    pub token: SessionToken,
}

/// Axum middleware: authenticate the `session` cookie and, for a mutating
/// method, verify the `X-CSRF-Token` header against it. Rejects with `401`
/// (no/invalid/expired session) or `403` (CSRF mismatch) before the wrapped
/// handler ever runs; on success, injects a [`CurrentSession`] the handler can
/// extract.
pub async fn require_session(
    State(sessions): State<SessionService>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = jar
        .get(SESSION_COOKIE)
        .map(|c| SessionToken::from_raw(c.value()))
    else {
        return unauthorized();
    };

    let authenticated = match sessions.authenticate(&token).await {
        Ok(a) => a,
        Err(SessionError::NotFound | SessionError::Expired) => return unauthorized(),
        Err(_) => return unauthorized(),
    };

    let is_mutating = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if is_mutating {
        let header_value = request
            .headers()
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok());
        if sessions.verify_csrf(&authenticated, header_value).is_err() {
            return forbidden();
        }
    }

    let mut request = request;
    request.extensions_mut().insert(CurrentSession {
        admin_id: authenticated.admin_id.as_str().to_string(),
        role: authenticated.role,
        token,
    });
    next.run(request).await
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: "unauthorized",
        }),
    )
        .into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorBody {
            error: "csrf_mismatch",
        }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

/// Build the `Set-Cookie` pair for a freshly issued session: the `HttpOnly`
/// session cookie and the client-readable CSRF cookie, both `Secure`,
/// `SameSite=Strict`, scoped to `/`, and capped at the session's absolute
/// expiry.
///
/// `SameSite=Strict` (rather than `Lax`) is safe here because nothing in this
/// admin panel needs the cookie sent on a cross-site top-level navigation: an
/// admin always reaches the app by typing the URL or via an already-open tab,
/// never by following a link from another site into an authenticated page.
pub fn attach_session_cookies(jar: CookieJar, issued: &IssuedSession) -> CookieJar {
    let expires = to_offset_date_time(issued.absolute_expires_at);

    let session_cookie = Cookie::build((SESSION_COOKIE, issued.token.as_str().to_string()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .expires(expires)
        .build();

    // Not `HttpOnly`: client-side script must read it to mirror it into the
    // `X-CSRF-Token` header on mutating requests.
    let csrf_cookie = Cookie::build((CSRF_COOKIE, issued.csrf_token.as_str().to_string()))
        .http_only(false)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .expires(expires)
        .build();

    jar.add(session_cookie).add(csrf_cookie)
}

/// Clear both session cookies (logout): same name/path/flags with an
/// already-expired deadline, which is what makes the removal stick across
/// browsers.
fn clear_session_cookies(jar: CookieJar) -> CookieJar {
    let expired = Cookie::build((SESSION_COOKIE, ""))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .expires(OffsetDateTime::UNIX_EPOCH)
        .build();
    let expired_csrf = Cookie::build((CSRF_COOKIE, ""))
        .http_only(false)
        .secure(true)
        .same_site(SameSite::Strict)
        .path("/")
        .expires(OffsetDateTime::UNIX_EPOCH)
        .build();
    jar.add(expired).add(expired_csrf)
}

fn to_offset_date_time(time: SystemTime) -> OffsetDateTime {
    OffsetDateTime::from(time)
}

/// The body `GET /auth/me` returns: enough for the frontend to know who is
/// logged in and which role's guards to apply.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MeOut {
    /// The authenticated administrator's opaque id.
    admin_id: String,
    /// The administrator's role (e.g. `admin`).
    role: String,
}

/// Handle `GET /auth/me`: report the authenticated admin's id and role, so
/// frontend guards can hide routes/actions the role does not permit without
/// a separate round trip. Any authenticated role may call this — it is not
/// itself role-gated (see [`crate::rbac::require_role`] for endpoints that
/// are).
#[utoipa::path(
    get,
    path = "/auth/me",
    responses(
        (status = 200, description = "The authenticated admin's id and role", body = MeOut),
        (status = 401, description = "No valid session"),
    ),
    tag = "auth",
)]
pub(crate) async fn me_handler(current: axum::Extension<CurrentSession>) -> impl IntoResponse {
    Json(MeOut {
        admin_id: current.0.admin_id.clone(),
        role: current.0.role.as_str().to_string(),
    })
}

/// Handle `POST /auth/logout`: revoke the session server-side, record the
/// event to the audit trail, and clear both cookies. Requires (via
/// [`require_session`]) a valid session and a matching CSRF header — logout
/// is a mutation like any other.
#[utoipa::path(
    post,
    path = "/auth/logout",
    responses(
        (status = 204, description = "Logged out; clears the session and csrf cookies"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Missing or mismatched CSRF header"),
    ),
    tag = "auth",
)]
pub(crate) async fn logout_handler(
    State(state): State<LogoutState>,
    current: axum::Extension<CurrentSession>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    jar: CookieJar,
) -> impl IntoResponse {
    // Best-effort: an already-gone session (e.g. it just expired) is not an
    // error from the caller's point of view — the outcome they wanted (no
    // longer logged in) already holds.
    let _ = state.sessions.revoke(&current.0.token).await;

    let event = NewAuditEvent {
        event_type: AuditEventType::LoggedOut,
        admin_id: Some(AdminId::new(current.0.admin_id.clone())),
        email_attempted: None,
        ip: client_ip,
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        occurred_at: SystemTime::now(),
    };
    // Best-effort, same reasoning as the login handler: an audit-store outage
    // must never block a logout.
    if let Err(e) = state.audit.record(event).await {
        eprintln!("logout: failed to record audit event: {e}");
    }

    (StatusCode::NO_CONTENT, clear_session_cookies(jar))
}
