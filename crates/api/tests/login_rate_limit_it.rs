//! End-to-end "basic load test" for the login rate limiter (bead
//! authapp-5af1bb): drives the real `/auth/login` router — wired to a real,
//! ephemeral, freshly migrated Postgres via `testkit`, no fakes — with a
//! burst of rapid requests from one client IP and confirms the request past
//! the configured limit gets `429` with `Retry-After`, while requests within
//! the limit are answered normally (`401` for bad credentials, since the rate
//! limit and the credential check are independent axes).

use std::sync::Arc;
use std::time::Duration;

use application::{AuditService, HealthService, LoginService, SessionService};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use infrastructure::{
    Argon2Hasher, Argon2Params, PgAdminRepository, PgAuditRepository, PgHealthCheck,
    PgIpLockoutStore, PgSessionRepository, SecureRandomTokens, SystemClock,
};
use tower::ServiceExt;

async fn router_with_rate_limit(pool: sqlx::PgPool, max_requests: u32) -> axum::Router {
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
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests,
            window: Duration::from_secs(60),
        },
    )
}

fn login_request(ip: &str, email: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", ip)
        .body(Body::from(
            serde_json::json!({"email": email, "password": "wrong-password"}).to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn requests_within_the_limit_reach_the_credential_check() {
    let db = testkit::spawn_test_db().await;
    let app = router_with_rate_limit(db.pool.clone(), 3).await;

    for i in 0..3 {
        let email = format!("nobody-{i}@example.com");
        let response = app
            .clone()
            .oneshot(login_request("203.0.113.10", &email))
            .await
            .unwrap();
        // Wrong credentials, but NOT rate-limited: the request reached the
        // (constant-time) credential check and was rejected on its merits.
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn the_request_past_the_limit_is_rejected_with_429_and_retry_after() {
    let db = testkit::spawn_test_db().await;
    let app = router_with_rate_limit(db.pool.clone(), 3).await;
    let ip = "203.0.113.20";

    // A distinct email per attempt isolates the IP's own budget from the
    // per-account budget — this test is about the IP axis.
    for i in 0..3 {
        let email = format!("nobody-{i}@example.com");
        let response = app
            .clone()
            .oneshot(login_request(ip, &email))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let fourth = app
        .clone()
        .oneshot(login_request(ip, "nobody-3@example.com"))
        .await
        .unwrap();
    assert_eq!(fourth.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = fourth
        .headers()
        .get("retry-after")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry_after <= 60);
}

#[tokio::test]
async fn a_different_client_ip_is_not_affected_by_another_ips_limit() {
    let db = testkit::spawn_test_db().await;
    let app = router_with_rate_limit(db.pool.clone(), 1).await;

    // Distinct emails per IP so only the IP axis is under test here — the
    // account axis has its own coverage in the account-key test below.
    let first_ip_first = app
        .clone()
        .oneshot(login_request("203.0.113.30", "alice@example.com"))
        .await
        .unwrap();
    assert_eq!(first_ip_first.status(), StatusCode::UNAUTHORIZED);
    let first_ip_second = app
        .clone()
        .oneshot(login_request("203.0.113.30", "bob@example.com"))
        .await
        .unwrap();
    assert_eq!(first_ip_second.status(), StatusCode::TOO_MANY_REQUESTS);

    // A different IP, with its own not-yet-seen email, has its own budget.
    let second_ip_first = app
        .clone()
        .oneshot(login_request("203.0.113.31", "carol@example.com"))
        .await
        .unwrap();
    assert_eq!(second_ip_first.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_same_account_from_different_ips_still_hits_the_account_limit() {
    let db = testkit::spawn_test_db().await;
    let app = router_with_rate_limit(db.pool.clone(), 1).await;

    let first = app
        .clone()
        .oneshot(login_request("203.0.113.40", "shared@example.com"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::UNAUTHORIZED);

    // A different IP targeting the SAME account still trips the account key,
    // even though that IP's own budget is untouched.
    let second = app
        .clone()
        .oneshot(login_request("203.0.113.41", "shared@example.com"))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}
