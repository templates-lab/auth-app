//! Integration test example for the audit module: exercises
//! [`PgAuditRepository`] against a real, ephemeral, freshly migrated Postgres
//! via `testkit`.

use std::time::{Duration, SystemTime};

use domain::{AdminId, AuditEventType, AuditRepository, NewAuditEvent};
use infrastructure::PgAuditRepository;

fn event(event_type: AuditEventType, occurred_at: SystemTime) -> NewAuditEvent {
    NewAuditEvent {
        event_type,
        admin_id: None,
        email_attempted: Some("admin@example.com".to_string()),
        ip: "203.0.113.1".to_string(),
        user_agent: Some("curl/8.0".to_string()),
        occurred_at,
    }
}

#[tokio::test]
async fn record_then_list_recent_round_trips() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAuditRepository::new(db.pool.clone());

    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    repo.record(&event(AuditEventType::LoginFailed, now))
        .await
        .unwrap();

    let recent = repo.list_recent(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].event_type, AuditEventType::LoginFailed);
    assert_eq!(
        recent[0].email_attempted.as_deref(),
        Some("admin@example.com")
    );
    assert_eq!(recent[0].ip, "203.0.113.1");
    assert_eq!(recent[0].user_agent.as_deref(), Some("curl/8.0"));
    assert_eq!(recent[0].occurred_at, now);
    assert!(recent[0].admin_id.is_none());
}

#[tokio::test]
async fn list_recent_orders_newest_first_and_respects_the_limit() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAuditRepository::new(db.pool.clone());
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);

    for (i, event_type) in [
        AuditEventType::LoginFailed,
        AuditEventType::LoginSucceeded,
        AuditEventType::LoggedOut,
    ]
    .into_iter()
    .enumerate()
    {
        repo.record(&event(event_type, base + Duration::from_secs(i as u64)))
            .await
            .unwrap();
    }

    let recent = repo.list_recent(2).await.unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].event_type, AuditEventType::LoggedOut);
    assert_eq!(recent[1].event_type, AuditEventType::LoginSucceeded);
}

#[tokio::test]
async fn admin_id_is_preserved_when_known() {
    let db = testkit::spawn_test_db().await;
    let repo = PgAuditRepository::new(db.pool.clone());

    // No FK to admin_users, so any well-formed UUID is accepted — the audit
    // trail must survive even if the admin row is later deleted.
    let admin_id = AdminId::new("00000000-0000-0000-0000-000000000001");
    let mut new_event = event(
        AuditEventType::LoginSucceeded,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
    );
    new_event.admin_id = Some(admin_id.clone());

    repo.record(&new_event).await.unwrap();

    let recent = repo.list_recent(1).await.unwrap();
    assert_eq!(recent[0].admin_id, Some(admin_id));
}
