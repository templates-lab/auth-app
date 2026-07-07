//! The payment-webhook use case: verify a provider's raw notification,
//! deduplicate it by event id, persist it for diagnostics/replay, and apply
//! the reported status change transactionally.
//!
//! [`WebhookService`] wires the [`WebhookVerifier`], [`WebhookEventStore`], and
//! [`PaymentRepository`] ports; it holds no HTTP or storage knowledge.

use std::sync::Arc;
use std::time::SystemTime;

use payments::{
    PaymentRepository, PaymentRepositoryError, WebhookError, WebhookEventStore, WebhookVerifier,
};

/// The outcome of handling one raw webhook, which the delivery layer maps to a
/// status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookOutcome {
    /// The event was accepted and its status change applied.
    Processed,
    /// A correctly-signed event that maps to no status change we act on, or a
    /// payment we don't know — recorded, but a no-op. Still a `2xx`.
    Ignored,
    /// The event id was already recorded; not acted on again. A `2xx` — the
    /// provider's redelivery has succeeded from its point of view.
    Duplicate,
    /// The signature was missing/invalid or the payload malformed. Rejected
    /// and recorded for audit; the delivery layer answers `400`.
    Rejected,
    /// An internal failure (storage). The delivery layer answers `5xx` so the
    /// provider retries.
    Error(String),
}

/// Application service for inbound payment webhooks.
#[derive(Clone)]
pub struct WebhookService {
    verifier: Arc<dyn WebhookVerifier>,
    store: Arc<dyn WebhookEventStore>,
    payments: Arc<dyn PaymentRepository>,
}

impl std::fmt::Debug for WebhookService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookService").finish_non_exhaustive()
    }
}

impl WebhookService {
    /// Assemble the service from its ports.
    pub fn new(
        verifier: Arc<dyn WebhookVerifier>,
        store: Arc<dyn WebhookEventStore>,
        payments: Arc<dyn PaymentRepository>,
    ) -> Self {
        Self {
            verifier,
            store,
            payments,
        }
    }

    /// Handle one raw webhook (the exact bytes received plus its signature
    /// header).
    pub async fn handle(&self, payload: &[u8], signature: Option<&str>) -> WebhookOutcome {
        // 1. Verify the signature over the *raw* bytes. A failure is recorded
        //    (audit trail) and rejected without ever being acted on.
        let event = match self.verifier.verify(payload, signature) {
            Ok(event) => event,
            Err(err) => {
                let reason = match err {
                    WebhookError::InvalidSignature => "invalid_signature",
                    WebhookError::Malformed(_) => "malformed",
                };
                if let Err(e) = self.store.record_rejected(payload, signature, reason).await {
                    tracing::warn!("webhook: failed to record rejected event: {e}");
                }
                return WebhookOutcome::Rejected;
            }
        };

        // 2. Deduplicate by event id (also persists the raw payload). A
        //    redelivery of an already-seen event is not acted on again.
        match self.store.record_and_claim(&event.event_id, payload).await {
            Ok(true) => {}
            Ok(false) => return WebhookOutcome::Duplicate,
            Err(e) => return WebhookOutcome::Error(e.to_string()),
        }

        // 3. Apply the reported status change, if it maps to a payment we know
        //    and is a legal transition.
        let (Some(reference), Some(next)) = (event.reference, event.new_status) else {
            return WebhookOutcome::Ignored;
        };
        let payment = match self.payments.find_by_provider_reference(&reference).await {
            Ok(Some(p)) => p,
            Ok(None) => return WebhookOutcome::Ignored,
            Err(e) => return WebhookOutcome::Error(e.to_string()),
        };
        if !payment.status.can_transition_to(next) {
            // Already applied, terminal, or an out-of-order delivery — the
            // stored state machine is the source of truth, so this is a no-op.
            return WebhookOutcome::Ignored;
        }
        match self
            .payments
            .transition(
                &payment.id,
                payment.status,
                next,
                Some("provider_webhook"),
                SystemTime::now(),
            )
            .await
        {
            Ok(()) => WebhookOutcome::Processed,
            // A concurrent transition already moved it; the guard did its job.
            Err(PaymentRepositoryError::Conflict) => WebhookOutcome::Ignored,
            Err(e) => WebhookOutcome::Error(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    use payments::{
        Currency, Money, NewPayment, Payment, PaymentId, PaymentStatus, PaymentStatusChange,
        ProviderReference, WebhookEvent, WebhookStoreError,
    };

    struct FakeVerifier {
        result: Mutex<Option<Result<WebhookEvent, WebhookError>>>,
    }
    impl FakeVerifier {
        fn ok(event: WebhookEvent) -> Self {
            Self {
                result: Mutex::new(Some(Ok(event))),
            }
        }
        fn err(err: WebhookError) -> Self {
            Self {
                result: Mutex::new(Some(Err(err))),
            }
        }
    }
    impl WebhookVerifier for FakeVerifier {
        fn verify(
            &self,
            _payload: &[u8],
            _signature: Option<&str>,
        ) -> Result<WebhookEvent, WebhookError> {
            match self.result.lock().unwrap().take() {
                Some(Ok(e)) => Ok(e),
                Some(Err(e)) => Err(e),
                None => panic!("verify called more than once"),
            }
        }
    }

    #[derive(Default)]
    struct InMemoryStore {
        claimed: Mutex<HashSet<String>>,
        rejected: Mutex<u32>,
    }
    #[async_trait::async_trait]
    impl WebhookEventStore for InMemoryStore {
        async fn record_rejected(
            &self,
            _payload: &[u8],
            _signature: Option<&str>,
            _reason: &str,
        ) -> Result<(), WebhookStoreError> {
            *self.rejected.lock().unwrap() += 1;
            Ok(())
        }
        async fn record_and_claim(
            &self,
            event_id: &str,
            _payload: &[u8],
        ) -> Result<bool, WebhookStoreError> {
            Ok(self.claimed.lock().unwrap().insert(event_id.to_string()))
        }
    }

    #[derive(Default)]
    struct InMemoryPayments {
        rows: Mutex<Vec<Payment>>,
    }
    #[async_trait::async_trait]
    impl PaymentRepository for InMemoryPayments {
        async fn insert(&self, _p: &NewPayment) -> Result<PaymentId, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn find(&self, _id: &PaymentId) -> Result<Option<Payment>, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn find_by_provider_reference(
            &self,
            reference: &ProviderReference,
        ) -> Result<Option<Payment>, PaymentRepositoryError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.provider_reference.as_ref() == Some(reference))
                .cloned())
        }
        async fn set_provider_reference(
            &self,
            _id: &PaymentId,
            _reference: &ProviderReference,
        ) -> Result<(), PaymentRepositoryError> {
            unimplemented!()
        }
        async fn transition(
            &self,
            id: &PaymentId,
            expected_current: PaymentStatus,
            next: PaymentStatus,
            _reason: Option<&str>,
            _occurred_at: SystemTime,
        ) -> Result<(), PaymentRepositoryError> {
            let mut rows = self.rows.lock().unwrap();
            let payment = rows
                .iter_mut()
                .find(|p| &p.id == id)
                .ok_or(PaymentRepositoryError::Conflict)?;
            if payment.status != expected_current {
                return Err(PaymentRepositoryError::Conflict);
            }
            payment.status = next;
            Ok(())
        }
        async fn history(
            &self,
            _id: &PaymentId,
        ) -> Result<Vec<PaymentStatusChange>, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn list(
            &self,
            _query: &payments::PaymentQuery,
        ) -> Result<Vec<Payment>, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn count(
            &self,
            _query: &payments::PaymentQuery,
        ) -> Result<u64, PaymentRepositoryError> {
            unimplemented!()
        }
    }

    fn payment(reference: &str, status: PaymentStatus) -> Payment {
        Payment {
            id: PaymentId::new(format!("id-{reference}")),
            provider_reference: Some(ProviderReference::new(reference)),
            amount: Money::from_minor_units(1000, Currency::parse("USD").unwrap()).unwrap(),
            status,
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn event(id: &str, reference: &str, status: PaymentStatus) -> WebhookEvent {
        WebhookEvent {
            event_id: id.to_string(),
            reference: Some(ProviderReference::new(reference)),
            new_status: Some(status),
        }
    }

    fn service(
        verifier: FakeVerifier,
        store: Arc<InMemoryStore>,
        payments: Arc<InMemoryPayments>,
    ) -> WebhookService {
        WebhookService::new(Arc::new(verifier), store, payments)
    }

    #[tokio::test]
    async fn invalid_signature_is_rejected_and_recorded() {
        let store = Arc::new(InMemoryStore::default());
        let svc = service(
            FakeVerifier::err(WebhookError::InvalidSignature),
            store.clone(),
            Arc::new(InMemoryPayments::default()),
        );
        assert_eq!(svc.handle(b"raw", None).await, WebhookOutcome::Rejected);
        assert_eq!(*store.rejected.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_valid_event_applies_the_transition() {
        let payments = Arc::new(InMemoryPayments::default());
        payments
            .rows
            .lock()
            .unwrap()
            .push(payment("pi_1", PaymentStatus::Authorized));
        let svc = service(
            FakeVerifier::ok(event("evt_1", "pi_1", PaymentStatus::Captured)),
            Arc::new(InMemoryStore::default()),
            payments.clone(),
        );
        assert_eq!(
            svc.handle(b"raw", Some("sig")).await,
            WebhookOutcome::Processed
        );
        assert_eq!(
            payments.rows.lock().unwrap()[0].status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn a_duplicate_event_id_is_not_acted_on_twice() {
        let payments = Arc::new(InMemoryPayments::default());
        payments
            .rows
            .lock()
            .unwrap()
            .push(payment("pi_1", PaymentStatus::Authorized));
        let store = Arc::new(InMemoryStore::default());

        let first = service(
            FakeVerifier::ok(event("evt_1", "pi_1", PaymentStatus::Captured)),
            store.clone(),
            payments.clone(),
        );
        assert_eq!(
            first.handle(b"raw", Some("sig")).await,
            WebhookOutcome::Processed
        );

        // A second delivery of the same event id: deduplicated, no double
        // effect (the payment stays Captured, not transitioned again).
        let second = service(
            FakeVerifier::ok(event("evt_1", "pi_1", PaymentStatus::Refunded)),
            store,
            payments.clone(),
        );
        assert_eq!(
            second.handle(b"raw", Some("sig")).await,
            WebhookOutcome::Duplicate
        );
        assert_eq!(
            payments.rows.lock().unwrap()[0].status,
            PaymentStatus::Captured
        );
    }

    #[tokio::test]
    async fn an_unknown_reference_is_ignored() {
        let svc = service(
            FakeVerifier::ok(event("evt_1", "pi_unknown", PaymentStatus::Captured)),
            Arc::new(InMemoryStore::default()),
            Arc::new(InMemoryPayments::default()),
        );
        assert_eq!(
            svc.handle(b"raw", Some("sig")).await,
            WebhookOutcome::Ignored
        );
    }

    #[tokio::test]
    async fn an_illegal_transition_is_ignored() {
        let payments = Arc::new(InMemoryPayments::default());
        // Already Refunded (terminal): a Captured event is not a legal move.
        payments
            .rows
            .lock()
            .unwrap()
            .push(payment("pi_1", PaymentStatus::Refunded));
        let svc = service(
            FakeVerifier::ok(event("evt_1", "pi_1", PaymentStatus::Captured)),
            Arc::new(InMemoryStore::default()),
            payments,
        );
        assert_eq!(
            svc.handle(b"raw", Some("sig")).await,
            WebhookOutcome::Ignored
        );
    }
}
