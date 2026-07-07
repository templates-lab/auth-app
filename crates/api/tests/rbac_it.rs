//! End-to-end test for RBAC (bead authapp-e00d47): drives the real router —
//! login, `/auth/me`, and the admin-gated `/audit/events` — against a real,
//! ephemeral, freshly migrated Postgres via `testkit`, with both an `admin`
//! and a `user` role account.
//!
//! Only bootstrap ever creates an account through the HTTP API, and it always
//! assigns `admin`; there is no "create user" endpoint yet (out of scope for
//! this bead — it is about the enforcement mechanism, not account
//! management), so the `user`-role account here is inserted directly through
//! [`PgAdminRepository`], the same as an operator or a future admin-panel
//! "invite a user" feature would.

use std::sync::Arc;
use std::time::Duration;

use application::{AuditService, BootstrapService, HealthService, LoginService, SessionService};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use domain::{AdminRepository, Email, NewAdmin, PasswordHasher, Role};
use infrastructure::{
    Argon2Hasher, Argon2Params, PgAdminRepository, PgAuditRepository, PgHealthCheck,
    PgIpLockoutStore, PgSessionRepository, SecureRandomTokens, SystemClock,
};
use tower::ServiceExt;

const ADMIN_EMAIL: &str = "admin@example.com";
const ADMIN_PASSWORD: &str = "Str0ngEnoughPassphrase!";
const USER_EMAIL: &str = "user@example.com";
const USER_PASSWORD: &str = "AlsoStr0ngEnoughPassphrase!";

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
        None,
        None,
        None,
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests: 100,
            window: Duration::from_secs(60),
        },
    )
}

/// Bootstrap the first (`admin`-role) account, then insert a second,
/// `user`-role account directly through the repository — there is no
/// "create user" HTTP endpoint yet, so this is how a `user`-role account
/// comes to exist for now.
async fn seed_admin_and_user(pool: sqlx::PgPool) {
    let hasher = Arc::new(Argon2Hasher::new(Argon2Params::owasp_default()).unwrap());
    let repo = Arc::new(PgAdminRepository::new(pool));

    BootstrapService::new(
        repo.clone(),
        hasher.clone(),
        domain::PasswordPolicy::recommended(),
    )
    .create_first_admin(ADMIN_EMAIL, ADMIN_PASSWORD)
    .await
    .unwrap();

    let user_password_hash = hasher.hash(USER_PASSWORD).await.unwrap();
    repo.insert(&NewAdmin {
        email: Email::parse(USER_EMAIL).unwrap(),
        password_hash: user_password_hash,
        role: Role::parse("user").unwrap(),
    })
    .await
    .unwrap();
}

fn login_body(email: &str, password: &str) -> Body {
    Body::from(serde_json::json!({"email": email, "password": password}).to_string())
}

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

async fn login_and_get_session_cookie(app: &axum::Router, email: &str, password: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body(email, password))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "login must succeed");
    cookie_value(&response, "session")
}

#[tokio::test]
async fn me_reports_the_authenticated_role() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let admin_session = login_and_get_session_cookie(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let admin_me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/me")
                .header("cookie", format!("session={admin_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_me.status(), StatusCode::OK);
    let body = axum::body::to_bytes(admin_me.into_body(), usize::MAX)
        .await
        .unwrap();
    let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["role"], "admin");

    let user_session = login_and_get_session_cookie(&app, USER_EMAIL, USER_PASSWORD).await;
    let user_me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/me")
                .header("cookie", format!("session={user_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(user_me.into_body(), usize::MAX)
        .await
        .unwrap();
    let me: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["role"], "user");
}

#[tokio::test]
async fn admin_gated_endpoint_rejects_a_user_role_session() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    // The admin can read the audit trail.
    let admin_session = login_and_get_session_cookie(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let admin_events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .header("cookie", format!("session={admin_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_events.status(), StatusCode::OK);

    // The user role — authenticated, but not admin — is rejected with 403,
    // not 401: the session itself is perfectly valid, it just lacks the role
    // this endpoint requires.
    let user_session = login_and_get_session_cookie(&app, USER_EMAIL, USER_PASSWORD).await;
    let user_events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/audit/events")
                .header("cookie", format!("session={user_session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(user_events.status(), StatusCode::FORBIDDEN);

    // Every error, whatever the endpoint, carries the same shape: a machine
    // code, a client-safe message, and a correlation trace id.
    let body = axum::body::to_bytes(user_events.into_body(), usize::MAX)
        .await
        .unwrap();
    let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(err["code"], "forbidden");
    assert!(err["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(err["trace_id"].as_str().is_some_and(|t| !t.is_empty()));
}

#[tokio::test]
async fn malformed_login_is_422_with_per_field_detail() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    // A malformed email and an empty password: a request-shape problem, so a
    // 422 with per-field messages — and it reveals nothing about any account.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("content-type", "application/json")
                .body(login_body("not-an-email", ""))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(err["code"], "validation_failed");
    assert!(err["fields"]["email"].as_str().is_some());
    assert!(err["fields"]["password"].as_str().is_some());
    assert!(err["trace_id"].as_str().is_some_and(|t| !t.is_empty()));
}
