//! The audit-trail use case: record an event, or list recent ones for
//! display.

use std::sync::Arc;

use domain::{AuditEvent, AuditRepository, NewAuditEvent, RepositoryError};

/// Application service exposing the audit trail.
///
/// Cloneable (an `Arc` inside), so the delivery layer can hold it as router
/// state alongside [`crate::LoginService`] and [`crate::SessionService`].
#[derive(Clone)]
pub struct AuditService {
    repo: Arc<dyn AuditRepository>,
}

impl std::fmt::Debug for AuditService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditService").finish_non_exhaustive()
    }
}

impl AuditService {
    /// Build the service from an adapter implementing the domain port.
    pub fn new(repo: Arc<dyn AuditRepository>) -> Self {
        Self { repo }
    }

    /// Record an event.
    pub async fn record(&self, event: NewAuditEvent) -> Result<(), RepositoryError> {
        self.repo.record(&event).await?;
        Ok(())
    }

    /// The most recent events, newest first, capped at `limit`.
    pub async fn recent(&self, limit: u32) -> Result<Vec<AuditEvent>, RepositoryError> {
        self.repo.list_recent(limit).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::SystemTime;

    use domain::AuditEventType;

    #[derive(Default)]
    struct InMemoryAudit {
        events: Mutex<Vec<NewAuditEvent>>,
    }

    #[async_trait::async_trait]
    impl AuditRepository for InMemoryAudit {
        async fn record(&self, event: &NewAuditEvent) -> Result<domain::AuditId, RepositoryError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(domain::AuditId::new("id-1"))
        }

        async fn list_recent(&self, limit: u32) -> Result<Vec<AuditEvent>, RepositoryError> {
            Ok(self
                .events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .take(limit as usize)
                .map(|e| AuditEvent {
                    id: domain::AuditId::new("id-1"),
                    event_type: e.event_type,
                    admin_id: e.admin_id.clone(),
                    email_attempted: e.email_attempted.clone(),
                    ip: e.ip.clone(),
                    user_agent: e.user_agent.clone(),
                    occurred_at: e.occurred_at,
                })
                .collect())
        }
    }

    fn event(event_type: AuditEventType) -> NewAuditEvent {
        NewAuditEvent {
            event_type,
            admin_id: None,
            email_attempted: Some("admin@example.com".to_string()),
            ip: "203.0.113.1".to_string(),
            user_agent: Some("curl/8.0".to_string()),
            occurred_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn record_then_recent_round_trips() {
        let service = AuditService::new(Arc::new(InMemoryAudit::default()));
        service
            .record(event(AuditEventType::LoginSucceeded))
            .await
            .unwrap();
        service
            .record(event(AuditEventType::LoggedOut))
            .await
            .unwrap();

        let recent = service.recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first.
        assert_eq!(recent[0].event_type, AuditEventType::LoggedOut);
        assert_eq!(recent[1].event_type, AuditEventType::LoginSucceeded);
    }

    #[tokio::test]
    async fn recent_respects_the_limit() {
        let service = AuditService::new(Arc::new(InMemoryAudit::default()));
        for _ in 0..5 {
            service
                .record(event(AuditEventType::LoginFailed))
                .await
                .unwrap();
        }

        assert_eq!(service.recent(2).await.unwrap().len(), 2);
    }
}
