//! Integration test example for the `admin_repo` module: exercises
//! [`PgAdminRepository`] against a real, ephemeral, freshly migrated Postgres
//! via `testkit`. Unlike the pure unit tests in `src/admin_repo.rs` (which
//! only cover epoch conversion), this proves the actual SQL round-trips.
//!
//! Each `#[tokio::test]` calls [`testkit::spawn_test_db`] itself, so every
//! test gets its own container and schema — no shared state, no ordering
//! dependence between tests. Requires a running Docker daemon.

use domain::{AdminRepository, Email, LockoutState, NewAdmin, PasswordHash, RepositoryError, Role};
use infrastructure::PgAdminRepository;

#[tokio::test]
async fn insert_then_find_by_email_round_trips() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAdminRepository::new(db.pool.clone());

    let email = Email::parse("admin@example.com").unwrap();
    let id = repo
        .insert(&NewAdmin {
            email: email.clone(),
            password_hash: PasswordHash::from_encoded("argon2-hash-placeholder"),
            role: Role::admin(),
        })
        .await
        .unwrap();

    let found = repo.find_by_email(&email).await.unwrap().unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.email, email);
    assert_eq!(found.password_hash.as_str(), "argon2-hash-placeholder");
    assert_eq!(found.lockout, LockoutState::clear());
}

#[tokio::test]
async fn find_by_email_is_none_for_unknown_address() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAdminRepository::new(db.pool.clone());

    let email = Email::parse("ghost@example.com").unwrap();
    assert!(repo.find_by_email(&email).await.unwrap().is_none());
}

#[tokio::test]
async fn duplicate_email_is_rejected() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAdminRepository::new(db.pool.clone());

    let email = Email::parse("admin@example.com").unwrap();
    let admin = NewAdmin {
        email,
        password_hash: PasswordHash::from_encoded("hash-1"),
        role: Role::admin(),
    };
    repo.insert(&admin).await.unwrap();

    let err = repo.insert(&admin).await.unwrap_err();
    assert!(matches!(err, RepositoryError::EmailTaken));
}

#[tokio::test]
async fn update_lockout_persists_counters() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAdminRepository::new(db.pool.clone());

    let email = Email::parse("admin@example.com").unwrap();
    let id = repo
        .insert(&NewAdmin {
            email: email.clone(),
            password_hash: PasswordHash::from_encoded("hash-1"),
            role: Role::admin(),
        })
        .await
        .unwrap();

    let locked = LockoutState {
        failed_attempts: 3,
        locked_until: Some(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000),
        ),
    };
    repo.update_lockout(&id, &locked).await.unwrap();

    let found = repo.find_by_email(&email).await.unwrap().unwrap();
    assert_eq!(found.lockout, locked);
}

#[tokio::test]
async fn count_reflects_inserted_admins() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAdminRepository::new(db.pool.clone());

    assert_eq!(repo.count().await.unwrap(), 0);

    repo.insert(&NewAdmin {
        email: Email::parse("one@example.com").unwrap(),
        password_hash: PasswordHash::from_encoded("hash-1"),
        role: Role::admin(),
    })
    .await
    .unwrap();
    repo.insert(&NewAdmin {
        email: Email::parse("two@example.com").unwrap(),
        password_hash: PasswordHash::from_encoded("hash-2"),
        role: Role::admin(),
    })
    .await
    .unwrap();

    assert_eq!(repo.count().await.unwrap(), 2);
}
