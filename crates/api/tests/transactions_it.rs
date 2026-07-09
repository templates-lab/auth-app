//! End-to-end test for the admin transactions view (bead authapp-a18fa6):
//! drives the real router — list, detail, and the admin-only refund — against a
//! real, ephemeral, freshly migrated Postgres via `testkit`.
//!
//! Payments are seeded straight through [`PgPaymentRepository`] (there is no
//! "create payment" HTTP endpoint), and the [`PaymentProvider`] is a small,
//! always-succeeding test double: the refund's provider call is unit-tested
//! against the real `FakePaymentProvider` elsewhere, so here it stands in so the
//! test can focus on the HTTP boundary, RBAC/CSRF, and persistence.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use application::{
    AuditService, BootstrapService, HealthService, LoginService, PaymentsService, SessionService,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use domain::{AdminRepository, Email, NewAdmin, PasswordHasher, Role};
use infrastructure::{
    Argon2Hasher, Argon2Params, PgAdminRepository, PgAuditRepository, PgHealthCheck,
    PgIpLockoutStore, PgPaymentRepository, PgSessionRepository, SecureRandomTokens, SystemClock,
};
use payments::{
    Currency, Money, NewPayment, PaymentId, PaymentProvider, PaymentRepository, PaymentStatus,
    ProviderError, ProviderIntent, ProviderReference,
};
use tower::ServiceExt;

const ADMIN_EMAIL: &str = "admin@example.com";
const ADMIN_PASSWORD: &str = "Str0ngEnoughPassphrase!";
const USER_EMAIL: &str = "user@example.com";
const USER_PASSWORD: &str = "AlsoStr0ngEnoughPassphrase!";

/// A payment provider whose `refund` always reports a full refund. The other
/// operations are unused by these tests.
struct AlwaysRefunds;

#[async_trait::async_trait]
impl PaymentProvider for AlwaysRefunds {
    async fn create_intent(&self, _amount: Money) -> Result<ProviderIntent, ProviderError> {
        unimplemented!()
    }
    async fn capture(
        &self,
        _reference: &ProviderReference,
        _amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        unimplemented!()
    }
    async fn refund(
        &self,
        reference: &ProviderReference,
        _amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        Ok(ProviderIntent {
            reference: reference.clone(),
            status: PaymentStatus::Refunded,
        })
    }
    async fn get_status(
        &self,
        _reference: &ProviderReference,
    ) -> Result<PaymentStatus, ProviderError> {
        unimplemented!()
    }
}

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
    let transactions = PaymentsService::new(
        Arc::new(PgPaymentRepository::new(pool.clone())),
        Arc::new(AlwaysRefunds),
    );
    api::router(
        health,
        login,
        sessions,
        audit,
        Arc::new(PgAdminRepository::new(pool.clone())),
        None,
        None,
        Some(transactions),
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests: 100,
            window: Duration::from_secs(60),
        },
    )
}

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

/// Seed one payment and walk it to `Captured` with a provider reference, so it
/// is refundable and has a multi-entry status history.
async fn seed_captured_payment(pool: sqlx::PgPool) -> PaymentId {
    let repo = PgPaymentRepository::new(pool);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let amount = Money::from_minor_units(2_500, Currency::parse("USD").unwrap()).unwrap();

    let id = repo
        .insert(&NewPayment {
            amount,
            created_at: now,
        })
        .await
        .unwrap();
    repo.set_provider_reference(&id, &ProviderReference::new("pi_seed"))
        .await
        .unwrap();
    repo.transition(
        &id,
        PaymentStatus::Created,
        PaymentStatus::Authorized,
        None,
        now,
    )
    .await
    .unwrap();
    repo.transition(
        &id,
        PaymentStatus::Authorized,
        PaymentStatus::Captured,
        None,
        now,
    )
    .await
    .unwrap();
    id
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

/// The `session` and `csrf` cookie values for a fresh login — both are needed
/// to authorize a mutating request (the CSRF token is echoed in a header).
async fn login(app: &axum::Router, email: &str, password: &str) -> (String, String) {
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
    (
        cookie_value(&response, "session"),
        cookie_value(&response, "csrf"),
    )
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn lists_transactions_with_total_and_status_filter() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    seed_captured_payment(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let (session, _csrf) = login(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/transactions?limit=10")
                .header("cookie", format!("session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = body_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["status"], "captured");
    assert_eq!(page["items"][0]["amount_minor_units"], 2_500);

    // A filter that matches nothing returns an empty page but a well-formed body.
    let empty = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/transactions?status=refunded")
                .header("cookie", format!("session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(empty).await["total"], 0);
}

#[tokio::test]
async fn detail_returns_the_full_status_history() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let id = seed_captured_payment(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let (session, _csrf) = login(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/transactions/{}", id.as_str()))
                .header("cookie", format!("session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = body_json(response).await;
    assert_eq!(detail["transaction"]["status"], "captured");
    // creation + Authorized + Captured = three recorded changes, oldest first.
    let history = detail["history"].as_array().unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0]["to"], "created");
    assert_eq!(history[2]["to"], "captured");

    // An unknown id is a 404.
    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/transactions/00000000-0000-0000-0000-000000000000")
                .header("cookie", format!("session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_can_refund_and_the_payment_becomes_refunded() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let id = seed_captured_payment(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let (session, csrf) = login(&app, ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/transactions/{}/refund", id.as_str()))
                .header("cookie", format!("session={session}; csrf={csrf}"))
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["status"], "refunded");

    // The refund persisted: the payment now reads back as refunded.
    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/transactions/{}", id.as_str()))
                .header("cookie", format!("session={session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(detail).await["transaction"]["status"], "refunded");
}

#[tokio::test]
async fn a_user_role_cannot_refund() {
    let db = testkit::spawn_test_db().await;
    seed_admin_and_user(db.pool.clone()).await;
    let id = seed_captured_payment(db.pool.clone()).await;
    let app = router(db.pool.clone()).await;

    let (session, csrf) = login(&app, USER_EMAIL, USER_PASSWORD).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/transactions/{}/refund", id.as_str()))
                .header("cookie", format!("session={session}; csrf={csrf}"))
                .header("x-csrf-token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Authenticated but not admin: 403, and the payment is untouched.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
