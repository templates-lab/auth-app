//! Integration tests for the OAuth Postgres adapters: the one-shot pending
//! auth store and the identity-linking repository, against a real, ephemeral,
//! freshly migrated Postgres via `testkit`.

use std::time::{Duration, SystemTime};

use domain::{
    AdminRepository, Email, NewAdmin, OAuthIdentityRepository, PasswordHash, PendingAuthStore,
    PendingAuthorization, ProviderId, Role,
};
use infrastructure::{PgAdminRepository, PgOAuthIdentityRepository, PgPendingAuthStore};

fn pending(state: &str) -> PendingAuthorization {
    PendingAuthorization {
        state: state.to_string(),
        provider: ProviderId::parse("google").unwrap(),
        nonce: "nonce-1".to_string(),
        code_verifier: "verifier-1".to_string(),
        redirect_uri: "https://app.example.com/cb".to_string(),
        created_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    }
}

#[tokio::test]
async fn pending_insert_then_consume_round_trips() {
    let db = testkit::spawn_test_db().await;
    let store = PgPendingAuthStore::new(db.pool.clone());

    store.insert(&pending("state-1")).await.unwrap();
    let consumed = store.consume("state-1").await.unwrap().unwrap();
    assert_eq!(consumed.provider, ProviderId::parse("google").unwrap());
    assert_eq!(consumed.nonce, "nonce-1");
    assert_eq!(consumed.code_verifier, "verifier-1");
    assert_eq!(consumed.redirect_uri, "https://app.example.com/cb");
}

#[tokio::test]
async fn consuming_a_state_is_one_shot() {
    let db = testkit::spawn_test_db().await;
    let store = PgPendingAuthStore::new(db.pool.clone());

    store.insert(&pending("state-1")).await.unwrap();
    assert!(store.consume("state-1").await.unwrap().is_some());
    // A replay finds nothing — the row was deleted by the first consume.
    assert!(store.consume("state-1").await.unwrap().is_none());
}

#[tokio::test]
async fn consuming_an_unknown_state_is_none() {
    let db = testkit::spawn_test_db().await;
    let store = PgPendingAuthStore::new(db.pool.clone());
    assert!(store.consume("never-inserted").await.unwrap().is_none());
}

async fn seed_admin(pool: &sqlx::PgPool, email: &str) -> domain::AdminId {
    PgAdminRepository::new(pool.clone())
        .insert(&NewAdmin {
            email: Email::parse(email).unwrap(),
            password_hash: PasswordHash::from_encoded("hash"),
            role: Role::admin(),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn identity_link_then_find_round_trips() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool, "admin@example.com").await;
    let repo = PgOAuthIdentityRepository::new(db.pool.clone());
    let google = ProviderId::parse("google").unwrap();

    assert!(repo.find_admin(&google, "sub-1").await.unwrap().is_none());

    repo.link(&google, "sub-1", "admin@example.com", &admin_id)
        .await
        .unwrap();
    assert_eq!(
        repo.find_admin(&google, "sub-1").await.unwrap(),
        Some(admin_id)
    );
}

#[tokio::test]
async fn linking_is_idempotent() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool, "admin@example.com").await;
    let repo = PgOAuthIdentityRepository::new(db.pool.clone());
    let google = ProviderId::parse("google").unwrap();

    repo.link(&google, "sub-1", "admin@example.com", &admin_id)
        .await
        .unwrap();
    // Re-linking the same identity refreshes the email rather than erroring.
    repo.link(&google, "sub-1", "new@example.com", &admin_id)
        .await
        .unwrap();
    assert_eq!(
        repo.find_admin(&google, "sub-1").await.unwrap(),
        Some(admin_id)
    );
}

#[tokio::test]
async fn deleting_the_admin_cascades_to_their_identities() {
    let db = testkit::spawn_test_db().await;
    let admin_id = seed_admin(&db.pool, "admin@example.com").await;
    let repo = PgOAuthIdentityRepository::new(db.pool.clone());
    let google = ProviderId::parse("google").unwrap();

    repo.link(&google, "sub-1", "admin@example.com", &admin_id)
        .await
        .unwrap();

    sqlx::query("DELETE FROM admin_users WHERE id = $1::uuid")
        .bind(admin_id.as_str())
        .execute(&db.pool)
        .await
        .unwrap();

    assert!(repo.find_admin(&google, "sub-1").await.unwrap().is_none());
}
