//! End-to-end test for the audit trail (bead authapp-c418dc): drives the real
//! router — login, the audit query endpoint, and logout — against a real,
//! ephemeral, freshly migrated Postgres via `testkit`, and confirms every
//! step is recorded and that reading the trail itself requires a session.

use std::sync::Arc;
use std::time::Duration;

use application::{AuditService, BootstrapService, HealthService, LoginService, SessionService};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use infrastructure::{
    Argon2Hasher, Argon2Params, PgAdminRepository, PgAuditRepository, PgHealthCheck,
    PgIpLockoutStore, PgSessionRepository, SecureRandomTokens, SystemClock,
};
use tower::ServiceExt;

const EMAIL: &str = "admin@example.com";
const PASSWORD: &str = "Str0ngEnoughPassphrase!";

async fn router(pool: sqlx::PgPool) -> axum::Router {
    let health = HealthService::new(Arc::new(PgHealthCheck::new(pool.clone())));
    let login = LoginService::new(
        Arc::new(PgAdminRepository::new(pool.clone())),
        Arc::new(PgIpLockoutStore::new(pool.clone())),
        Arc::new(Argon2Hasher::new(Argon2Params::owasp_default()).unwrap()),
        Arc::new(SystemClock),
        domain::LockoutPolicy::recommended(),
    );
    let sessions = SessionService::new(
        Arc::new(PgSessionRepository::new(pool.clone())),
        Arc::new(SecureRandomTokens),
        Arc::new(SystemClock),
        domain::SessionPolicy::recommended(),
    );
    let audit = AuditService::new(Arc::new(PgAuditRepository::new(pool.clone())));
    api::router(
        health,
        login,
        sessions,
        audit,
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests: 100,
            window: Duration::from_secs(60),
        },
    )
}

async fn bootstrap_admin(pool: sqlx::PgPool) {
    let bootstrap = BootstrapService::new(
        Arc::new(PgAdminRepository::new(pool)),
        Arc::new(Argon2Hasher::new(Argon2Params::owasp_default()).unwrap()),
        domain::PasswordPolicy::recommended(),
    );
    bootstrap.create_first_admin(EMAIL, PASSWORD).await.unwrap();
}

fn login_body() -> Body {
    Body::from(serde_json::json!({"email": EMAIL, "password": PASSWORD}).to_string())
}

/// Pull `name=value` out of one or more `Set-Cookie` response headers.
fn cookie_value(response: &axum::http::Response<Body>, name: &str) -> String {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            let (pair, _rest) = s.split_once(';')?;
            let (k, v) = pair.split_once('=')?;
            (k == name).then(|| v.to_string())
        })
        .unwrap_or_else(|| panic!("no {name} cookie in response"))
}

#[tokio::test]
async fn audit_events_require_a_session_and_record_login_success() {
    let db = testkit::spawn_test_db().await;
    bootstrap_admin(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    // Unauthenticated: the trail itself is gated.
    let unauthenticated = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    // Log in successfully.
    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_response.status(), StatusCode::OK);
    let session_cookie = cookie_value(&login_response, "session");

    // Authenticated: the trail includes the login we just performed.
    let events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .header("cookie", format!("session={session_cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(events_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(events_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let events = events.as_array().unwrap();
    assert!(events
        .iter()
        .any(|e| e["event_type"] == "login_succeeded" && e["email_attempted"] == EMAIL));
}

#[tokio::test]
async fn failed_login_is_recorded_without_leaking_a_password() {
    let db = testkit::spawn_test_db().await;
    bootstrap_admin(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    // A wrong password.
    let failed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"email": EMAIL, "password": "wrong"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(failed.status(), StatusCode::UNAUTHORIZED);

    // A real session, so the events endpoint answers.
    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body())
                .unwrap(),
        )
        .await
        .unwrap();
    let session_cookie = cookie_value(&login_response, "session");

    let events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .header("cookie", format!("session={session_cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(events_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let events = events.as_array().unwrap();

    let failed_event = events
        .iter()
        .find(|e| e["event_type"] == "login_failed")
        .expect("the failed login must be recorded");
    // No password anywhere in the record — only the JSON keys this type
    // defines exist, and none of them can hold one (see `NewAuditEvent`).
    let serialized = failed_event.to_string();
    assert!(!serialized.contains("wrong"));
}

#[tokio::test]
async fn logout_is_recorded() {
    let db = testkit::spawn_test_db().await;
    bootstrap_admin(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body())
                .unwrap(),
        )
        .await
        .unwrap();
    let session_cookie = cookie_value(&login_response, "session");
    let csrf_cookie = cookie_value(&login_response, "csrf");

    let logout_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header(
                    "cookie",
                    format!("session={session_cookie}; csrf={csrf_cookie}"),
                )
                .header("x-csrf-token", &csrf_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout_response.status(), StatusCode::NO_CONTENT);

    // Log in again (the old session is gone) so the events endpoint answers.
    let second_login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body())
                .unwrap(),
        )
        .await
        .unwrap();
    let second_session_cookie = cookie_value(&second_login, "session");

    let events_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .header("cookie", format!("session={second_session_cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(events_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let events = events.as_array().unwrap();
    assert!(events.iter().any(|e| e["event_type"] == "logged_out"));
}
