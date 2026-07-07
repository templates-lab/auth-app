//! Integration test example for the sessions module: exercises
//! [`PgSessionRepository`] against a real, ephemeral, freshly migrated
//! Postgres via `testkit`. A session references an admin row (foreign key),
//! so each test first inserts one through [`PgAdminRepository`].
//!
//! Each `#[tokio::test]` calls [`testkit::spawn_test_db`] itself, so every
//! test gets its own container and schema. Requires a running Docker daemon.

use std::time::{Duration, SystemTime};

use domain::{
    AdminRepository, CsrfToken, Email, NewAdmin, PasswordHash, Role, Session, SessionRepository,
    SessionToken,
};
use infrastructure::{PgAdminRepository, PgSessionRepository};

/// Insert a throwaway admin and return its id, so session tests have a valid
/// foreign key to attach to.
async fn seed_admin(pool: &sqlx::PgPool) -> domain::AdminId {
    PgAdminRepository::new(pool.clone())
        .insert(&NewAdmin {
            email: Email::parse("admin@example.com").unwrap(),
            password_hash: PasswordHash::from_encoded("hash-1"),
            role: Role::admin(),
        })
        .await
        .unwrap()
}

fn sample_session(admin_id: domain::AdminId, now: SystemTime) -> Session {
    Session {
        token: SessionToken::from_raw("test-token-1"),
        admin_id,
        role: Role::admin(),
        csrf_token: CsrfToken::from_raw("test-csrf-1"),
        created_at: now,
        absolute_expires_at: now + Duration::from_secs(3600),
        last_seen_at: now,
    }
}

#[tokio::test]
async fn insert_then_find_round_trips() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool).await;
    let repo = PgSessionRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let session = sample_session(admin_id.clone(), now);
    repo.insert(&session).await.unwrap();

    let found = repo.find(&session.token).await.unwrap().unwrap();
    assert_eq!(found.admin_id, admin_id);
    assert_eq!(found.csrf_token.as_str(), "test-csrf-1");
    assert_eq!(found.created_at, now);
    assert_eq!(found.last_seen_at, now);
    assert_eq!(found.absolute_expires_at, now + Duration::from_secs(3600));
}

#[tokio::test]
async fn find_is_none_for_unknown_token() {
    let db = testkit::spawn_test_db().await;
    let repo = PgSessionRepository::new(db.pool.clone());

    let missing = SessionToken::from_raw("nonexistent");
    assert!(repo.find(&missing).await.unwrap().is_none());
}

#[tokio::test]
async fn touch_advances_last_seen_at() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool).await;
    let repo = PgSessionRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let session = sample_session(admin_id, now);
    repo.insert(&session).await.unwrap();

    let later = now + Duration::from_secs(120);
    repo.touch(&session.token, later).await.unwrap();

    let found = repo.find(&session.token).await.unwrap().unwrap();
    assert_eq!(found.last_seen_at, later);
    // Untouched fields survive the update.
    assert_eq!(found.absolute_expires_at, session.absolute_expires_at);
}

#[tokio::test]
async fn delete_removes_the_session() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool).await;
    let repo = PgSessionRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let session = sample_session(admin_id, now);
    repo.insert(&session).await.unwrap();

    repo.delete(&session.token).await.unwrap();
    assert!(repo.find(&session.token).await.unwrap().is_none());

    // Deleting an already-gone session is not an error (logout is idempotent).
    repo.delete(&session.token).await.unwrap();
}

#[tokio::test]
async fn deleting_the_admin_cascades_to_their_sessions() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool).await;
    let repo = PgSessionRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let session = sample_session(admin_id.clone(), now);
    repo.insert(&session).await.unwrap();

    sqlx::query("DELETE FROM admin_users WHERE id = $1::uuid")
        .bind(admin_id.as_str())
        .execute(&db.pool)
        .await
        .unwrap();

    assert!(repo.find(&session.token).await.unwrap().is_none());
}
