//! HTTP boundary for administrator login.
//!
//! Translates `POST /auth/login` into a call on the application
//! [`LoginService`] and maps the outcome back to a status code. It holds no
//! business logic: every security decision (constant-time verification,
//! lockout) lives in the application and domain layers; this module only speaks
//! HTTP.

use std::convert::Infallible;
use std::net::SocketAddr;

use application::{LoginError, LoginRequest, LoginService};
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

/// Mount the auth routes with the login service as their state.
pub fn routes(login: LoginService) -> Router {
    Router::new()
        .route("/auth/login", post(login_handler))
        .with_state(login)
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
struct ClientIp(String);

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
/// `200` with the admin id on success; `401` for invalid credentials (identical
/// for a wrong password and a nonexistent account); `429` with `Retry-After`
/// when the account or IP is locked out; `500` on an internal failure.
async fn login_handler(
    State(login): State<LoginService>,
    ClientIp(client_ip): ClientIp,
    Json(body): Json<LoginBody>,
) -> Response {
    let request = LoginRequest {
        email: body.email,
        password: body.password,
        client_ip,
    };

    match login.login(request).await {
        Ok(id) => (
            StatusCode::OK,
            Json(LoginOk {
                admin_id: id.as_str().to_string(),
            }),
        )
            .into_response(),
        Err(LoginError::InvalidCredentials) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "invalid_credentials",
            }),
        )
            .into_response(),
        Err(LoginError::TooManyAttempts { retry_after_secs }) => {
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
