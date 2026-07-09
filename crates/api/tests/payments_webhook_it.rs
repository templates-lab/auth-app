//! End-to-end test for payment webhooks (bead authapp-c78901): drives
//! `POST /payments/webhooks` through the real router — real Stripe signature
//! verifier, real Postgres webhook store, real payment repository — against an
//! ephemeral, freshly migrated database. Covers all three acceptance criteria:
//! an invalid signature is 400'd and audited, a duplicate event id has no
//! double effect, and raw events are persisted for diagnostics/replay.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use application::{AuditService, HealthService, LoginService, SessionService, WebhookService};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, Mac};
use infrastructure::{
    Argon2Hasher, Argon2Params, PgAdminRepository, PgAuditRepository, PgHealthCheck,
    PgIpLockoutStore, PgPaymentRepository, PgSessionRepository, PgWebhookEventStore,
    SecureRandomTokens, StripeWebhookConfig, StripeWebhookVerifier, SystemClock,
};
use payments::{Currency, Money, NewPayment, PaymentRepository, PaymentStatus, ProviderReference};
use sha2::Sha256;
use tower::ServiceExt;

const SECRET: &str = "whsec_test_secret";

fn usd(minor: i64) -> Money {
    Money::from_minor_units(minor, Currency::parse("USD").unwrap()).unwrap()
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
    let webhooks = WebhookService::new(
        Arc::new(StripeWebhookVerifier::new(StripeWebhookConfig::new(SECRET))),
        Arc::new(PgWebhookEventStore::new(pool.clone())),
        Arc::new(PgPaymentRepository::new(pool.clone())),
    );
    api::router(
        health,
        login,
        sessions,
        audit,
        Arc::new(PgAdminRepository::new(pool.clone())),
        None,
        Some(webhooks),
        None,
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests: 100,
            window: Duration::from_secs(60),
        },
    )
}

/// Seed a payment that already has a provider reference and is Authorized, so a
/// `payment_intent.succeeded` webhook can capture it.
async fn seed_authorized_payment(pool: &sqlx::PgPool, reference: &str) -> payments::PaymentId {
    let repo = PgPaymentRepository::new(pool.clone());
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(5000),
            created_at: now,
        })
        .await
        .unwrap();
    repo.set_provider_reference(&id, &ProviderReference::new(reference))
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
    id
}

fn succeeded_body(event_id: &str, intent: &str) -> Vec<u8> {
    serde_json::json!({
        "id": event_id,
        "type": "payment_intent.succeeded",
        "data": {"object": {"id": intent}}
    })
    .to_string()
    .into_bytes()
}

/// A valid `Stripe-Signature` header for `body` at the current time.
fn sign(body: &[u8]) -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("t={t},v1={hex}")
}

fn webhook_request(body: Vec<u8>, signature: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/payments/webhooks")
        .header("content-type", "application/json");
    if let Some(sig) = signature {
        builder = builder.header("stripe-signature", sig);
    }
    builder.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn a_valid_webhook_captures_the_payment_and_persists_the_raw_event() {
    let db = testkit::spawn_test_db().await;
    let id = seed_authorized_payment(&db.pool, "pi_1").await;
    let app = router(db.pool.clone()).await;

    let body = succeeded_body("evt_1", "pi_1");
    let sig = sign(&body);
    let response = app
        .oneshot(webhook_request(body.clone(), Some(&sig)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The payment moved Authorized -> Captured.
    let payment = PgPaymentRepository::new(db.pool.clone())
        .find(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);

    // The raw event was persisted (diagnostics/replay).
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT payload FROM payments.webhook_events WHERE event_id = $1")
            .bind("evt_1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(stored, body);
}

#[tokio::test]
async fn an_invalid_signature_is_rejected_with_400_and_audited() {
    let db = testkit::spawn_test_db().await;
    seed_authorized_payment(&db.pool, "pi_1").await;
    let app = router(db.pool.clone()).await;

    let body = succeeded_body("evt_1", "pi_1");
    let response = app
        .oneshot(webhook_request(body, Some("t=1,v1=deadbeef")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // The rejected receipt was recorded for audit.
    let rejected: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payments.webhook_events WHERE NOT accepted")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(rejected, 1);
}

#[tokio::test]
async fn a_duplicate_event_has_no_double_effect() {
    let db = testkit::spawn_test_db().await;
    let id = seed_authorized_payment(&db.pool, "pi_1").await;
    let app = router(db.pool.clone()).await;

    let body = succeeded_body("evt_1", "pi_1");
    let sig = sign(&body);

    // First delivery: captures the payment.
    let first = app
        .clone()
        .oneshot(webhook_request(body.clone(), Some(&sig)))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    // Second delivery of the SAME event id: still 200, but deduplicated — no
    // second transition, and only one accepted receipt logged.
    let second = app
        .oneshot(webhook_request(body, Some(&sig)))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);

    let payment = PgPaymentRepository::new(db.pool.clone())
        .find(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);

    let accepted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments.webhook_events WHERE event_id = $1 AND accepted",
    )
    .bind("evt_1")
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(accepted, 1);

    // And the payment's own history recorded the capture exactly once.
    let captures: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments.payment_status_history \
         WHERE payment_id = $1::uuid AND to_status = 'captured'",
    )
    .bind(id.as_str())
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(captures, 1);
}
