//! Integration test example for the payments module: exercises
//! [`PgPaymentRepository`] — including the transactional, optimistic-
//! concurrency-guarded status transition — against a real, ephemeral,
//! freshly migrated Postgres via `testkit`.

use std::time::{Duration, SystemTime};

use infrastructure::PgPaymentRepository;
use payments::{
    Currency, Money, NewPayment, PaymentRepository, PaymentRepositoryError, PaymentStatus,
    ProviderReference,
};

fn usd(minor_units: i64) -> Money {
    Money::from_minor_units(minor_units, Currency::parse("USD").unwrap()).unwrap()
}

#[tokio::test]
async fn insert_then_find_round_trips_and_records_creation_history() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(2_500),
            created_at: now,
        })
        .await
        .unwrap();

    let found = repo.find(&id).await.unwrap().unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.amount, usd(2_500));
    assert_eq!(found.status, PaymentStatus::Created);
    assert!(found.provider_reference.is_none());
    assert_eq!(found.created_at, now);

    let history = repo.history(&id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].from, None);
    assert_eq!(history[0].to, PaymentStatus::Created);
}

#[tokio::test]
async fn find_is_none_for_unknown_id() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let missing = payments::PaymentId::new("00000000-0000-0000-0000-000000000000");
    assert!(repo.find(&missing).await.unwrap().is_none());
}

#[tokio::test]
async fn set_provider_reference_persists() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
        .await
        .unwrap();

    let reference = ProviderReference::new("pi_test_123");
    repo.set_provider_reference(&id, &reference).await.unwrap();

    let found = repo.find(&id).await.unwrap().unwrap();
    assert_eq!(found.provider_reference, Some(reference));
}

#[tokio::test]
async fn transition_updates_status_and_appends_history() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
        .await
        .unwrap();

    let later = now + Duration::from_secs(60);
    repo.transition(
        &id,
        PaymentStatus::Created,
        PaymentStatus::Authorized,
        None,
        later,
    )
    .await
    .unwrap();

    let found = repo.find(&id).await.unwrap().unwrap();
    assert_eq!(found.status, PaymentStatus::Authorized);
    assert_eq!(found.updated_at, later);

    let history = repo.history(&id).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].from, Some(PaymentStatus::Created));
    assert_eq!(history[1].to, PaymentStatus::Authorized);
}

#[tokio::test]
async fn transition_with_a_reason_is_recorded() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
        .await
        .unwrap();

    repo.transition(
        &id,
        PaymentStatus::Created,
        PaymentStatus::Failed,
        Some("card_declined"),
        now,
    )
    .await
    .unwrap();

    let history = repo.history(&id).await.unwrap();
    assert_eq!(history[1].reason.as_deref(), Some("card_declined"));
}

#[tokio::test]
async fn transition_rejects_a_stale_expected_current_status() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
        .await
        .unwrap();

    // First transition succeeds and moves the payment to `Authorized`.
    repo.transition(
        &id,
        PaymentStatus::Created,
        PaymentStatus::Authorized,
        None,
        now,
    )
    .await
    .unwrap();

    // A second caller still expecting `Created` (e.g. it read the row before
    // the first transition committed) loses the race.
    let err = repo
        .transition(
            &id,
            PaymentStatus::Created,
            PaymentStatus::Canceled,
            None,
            now,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, PaymentRepositoryError::Conflict));

    // The rejected transition left no trace: status and history are exactly
    // what the first, successful transition produced.
    let found = repo.find(&id).await.unwrap().unwrap();
    assert_eq!(found.status, PaymentStatus::Authorized);
    assert_eq!(repo.history(&id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn history_is_ordered_oldest_first() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let id = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
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

    let history = repo.history(&id).await.unwrap();
    let statuses: Vec<_> = history.iter().map(|h| h.to).collect();
    assert_eq!(
        statuses,
        [
            PaymentStatus::Created,
            PaymentStatus::Authorized,
            PaymentStatus::Captured,
        ]
    );
}

#[tokio::test]
async fn provider_reference_is_unique_across_payments() {
    let db = testkit::spawn_test_db().await;
    let repo = PgPaymentRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let first = repo
        .insert(&NewPayment {
            amount: usd(1_000),
            created_at: now,
        })
        .await
        .unwrap();
    let second = repo
        .insert(&NewPayment {
            amount: usd(2_000),
            created_at: now,
        })
        .await
        .unwrap();

    let reference = ProviderReference::new("pi_shared");
    repo.set_provider_reference(&first, &reference)
        .await
        .unwrap();

    let err = repo.set_provider_reference(&second, &reference).await;
    assert!(
        err.is_err(),
        "a duplicate provider reference must be rejected"
    );
}
