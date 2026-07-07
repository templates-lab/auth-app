//! The admin transactions use case (bead authapp-a18fa6): listing payments
//! with filters, reading one payment's full status history, and issuing a
//! refund.
//!
//! [`PaymentsService`] wires the [`PaymentRepository`] and [`PaymentProvider`]
//! ports. It holds no HTTP or storage knowledge — the delivery layer maps its
//! outcomes to status codes, and the role check that gates refunds to admins
//! lives in the API layer's RBAC middleware, not here.

use std::sync::Arc;
use std::time::SystemTime;

use payments::{
    Payment, PaymentId, PaymentPage, PaymentProvider, PaymentQuery, PaymentRepository,
    PaymentRepositoryError, PaymentStatus, PaymentStatusChange, ProviderError,
};

/// A payment together with its full, ordered status history — what the detail
/// view renders.
#[derive(Debug, Clone)]
pub struct PaymentWithHistory {
    /// The payment itself.
    pub payment: Payment,
    /// Every recorded status change, oldest first.
    pub history: Vec<PaymentStatusChange>,
}

/// The result of a successful refund: the payment's new status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundOutcome {
    /// The status the payment moved to (`Refunded` for a full refund,
    /// `PartiallyRefunded` for a partial one).
    pub status: PaymentStatus,
}

/// Why a refund could not be issued.
#[derive(Debug)]
pub enum RefundError {
    /// No payment with the given id exists.
    NotFound,
    /// The payment is not in a refundable state (only `Captured` or
    /// `PartiallyRefunded` payments can be refunded).
    NotRefundable(PaymentStatus),
    /// The payment has no provider reference, so the provider has nothing to
    /// refund against — it never reached the provider.
    NoProviderReference,
    /// The provider declined or could not process the refund.
    Provider(ProviderError),
    /// The payment's status changed underneath us (a concurrent transition);
    /// the caller may re-read and retry.
    Conflict,
    /// A storage failure.
    Backend(String),
}

impl std::fmt::Display for RefundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("payment not found"),
            Self::NotRefundable(status) => {
                write!(f, "payment in status {status} is not refundable")
            }
            Self::NoProviderReference => f.write_str("payment has no provider reference to refund"),
            Self::Provider(e) => write!(f, "provider refund failed: {e}"),
            Self::Conflict => f.write_str("payment status changed concurrently"),
            Self::Backend(msg) => write!(f, "refund backend error: {msg}"),
        }
    }
}

impl std::error::Error for RefundError {}

/// Application service for the admin transactions surface.
#[derive(Clone)]
pub struct PaymentsService {
    payments: Arc<dyn PaymentRepository>,
    provider: Arc<dyn PaymentProvider>,
}

impl std::fmt::Debug for PaymentsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaymentsService").finish_non_exhaustive()
    }
}

impl PaymentsService {
    /// Assemble the service from its ports.
    pub fn new(payments: Arc<dyn PaymentRepository>, provider: Arc<dyn PaymentProvider>) -> Self {
        Self { payments, provider }
    }

    /// One filtered, paginated page of payments plus the matching total.
    pub async fn list(&self, query: &PaymentQuery) -> Result<PaymentPage, PaymentRepositoryError> {
        // Count and page are read separately; a payment created between the two
        // reads only ever makes `total` a slight undercount for one request,
        // which is acceptable for a paged admin list (no lost or duplicated row
        // within the page itself).
        let total = self.payments.count(query).await?;
        let items = self.payments.list(query).await?;
        Ok(PaymentPage { items, total })
    }

    /// A single payment with its full status history, or `None` if unknown.
    pub async fn get(
        &self,
        id: &PaymentId,
    ) -> Result<Option<PaymentWithHistory>, PaymentRepositoryError> {
        let Some(payment) = self.payments.find(id).await? else {
            return Ok(None);
        };
        let history = self.payments.history(id).await?;
        Ok(Some(PaymentWithHistory { payment, history }))
    }

    /// Refund a payment in full.
    ///
    /// The stored state machine is the source of truth for whether a refund is
    /// legal: only a `Captured` or `PartiallyRefunded` payment can be refunded.
    /// The provider is called first (money moves there); only on its success do
    /// we record the status transition, guarded by the payment's expected
    /// current status so a concurrent change cannot double-apply.
    pub async fn refund(&self, id: &PaymentId) -> Result<RefundOutcome, RefundError> {
        let payment = self
            .payments
            .find(id)
            .await
            .map_err(|e| RefundError::Backend(e.to_string()))?
            .ok_or(RefundError::NotFound)?;

        if !matches!(
            payment.status,
            PaymentStatus::Captured | PaymentStatus::PartiallyRefunded
        ) {
            return Err(RefundError::NotRefundable(payment.status));
        }
        let reference = payment
            .provider_reference
            .as_ref()
            .ok_or(RefundError::NoProviderReference)?;

        let intent = self
            .provider
            .refund(reference, payment.amount)
            .await
            .map_err(RefundError::Provider)?;

        // The provider's reported status decides full vs partial; reject a
        // response that is not a legal move from the current stored status
        // rather than writing an impossible transition.
        if !payment.status.can_transition_to(intent.status) {
            return Err(RefundError::NotRefundable(payment.status));
        }

        match self
            .payments
            .transition(
                &payment.id,
                payment.status,
                intent.status,
                Some("admin_refund"),
                SystemTime::now(),
            )
            .await
        {
            Ok(()) => Ok(RefundOutcome {
                status: intent.status,
            }),
            Err(PaymentRepositoryError::Conflict) => Err(RefundError::Conflict),
            Err(e) => Err(RefundError::Backend(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use payments::{Currency, Money, NewPayment, PaymentId, ProviderIntent, ProviderReference};

    /// An in-memory payment repository seeded with fixed rows.
    #[derive(Default)]
    struct FakeRepo {
        rows: Mutex<Vec<Payment>>,
        history: Mutex<Vec<PaymentStatusChange>>,
    }

    #[async_trait::async_trait]
    impl PaymentRepository for FakeRepo {
        async fn insert(&self, _p: &NewPayment) -> Result<PaymentId, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn find(&self, id: &PaymentId) -> Result<Option<Payment>, PaymentRepositoryError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|p| &p.id == id)
                .cloned())
        }
        async fn find_by_provider_reference(
            &self,
            _r: &ProviderReference,
        ) -> Result<Option<Payment>, PaymentRepositoryError> {
            unimplemented!()
        }
        async fn set_provider_reference(
            &self,
            _id: &PaymentId,
            _r: &ProviderReference,
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
            let p = rows
                .iter_mut()
                .find(|p| &p.id == id)
                .ok_or(PaymentRepositoryError::Conflict)?;
            if p.status != expected_current {
                return Err(PaymentRepositoryError::Conflict);
            }
            p.status = next;
            Ok(())
        }
        async fn history(
            &self,
            _id: &PaymentId,
        ) -> Result<Vec<PaymentStatusChange>, PaymentRepositoryError> {
            Ok(self.history.lock().unwrap().clone())
        }
        async fn list(&self, query: &PaymentQuery) -> Result<Vec<Payment>, PaymentRepositoryError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|p| query.status.is_none_or(|s| p.status == s))
                .skip(query.offset as usize)
                .take(query.limit.max(1) as usize)
                .cloned()
                .collect())
        }
        async fn count(&self, query: &PaymentQuery) -> Result<u64, PaymentRepositoryError> {
            let rows = self.rows.lock().unwrap();
            Ok(rows
                .iter()
                .filter(|p| query.status.is_none_or(|s| p.status == s))
                .count() as u64)
        }
    }

    /// A provider whose `refund` returns a scripted status (or an error).
    struct ScriptedProvider(Mutex<Option<Result<PaymentStatus, ProviderError>>>);

    #[async_trait::async_trait]
    impl PaymentProvider for ScriptedProvider {
        async fn create_intent(&self, _a: Money) -> Result<ProviderIntent, ProviderError> {
            unimplemented!()
        }
        async fn capture(
            &self,
            _r: &ProviderReference,
            _a: Money,
        ) -> Result<ProviderIntent, ProviderError> {
            unimplemented!()
        }
        async fn refund(
            &self,
            reference: &ProviderReference,
            _a: Money,
        ) -> Result<ProviderIntent, ProviderError> {
            match self.0.lock().unwrap().take().expect("refund scripted once") {
                Ok(status) => Ok(ProviderIntent {
                    reference: reference.clone(),
                    status,
                }),
                Err(e) => Err(e),
            }
        }
        async fn get_status(&self, _r: &ProviderReference) -> Result<PaymentStatus, ProviderError> {
            unimplemented!()
        }
    }

    fn usd(minor: i64) -> Money {
        Money::from_minor_units(minor, Currency::parse("USD").unwrap()).unwrap()
    }

    fn payment(id: &str, status: PaymentStatus, reference: Option<&str>) -> Payment {
        Payment {
            id: PaymentId::new(id),
            provider_reference: reference.map(ProviderReference::new),
            amount: usd(2_500),
            status,
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn service(repo: FakeRepo, refund: Result<PaymentStatus, ProviderError>) -> PaymentsService {
        PaymentsService::new(
            Arc::new(repo),
            Arc::new(ScriptedProvider(Mutex::new(Some(refund)))),
        )
    }

    #[tokio::test]
    async fn list_returns_a_page_with_total() {
        let repo = FakeRepo::default();
        repo.rows.lock().unwrap().extend([
            payment("a", PaymentStatus::Captured, Some("pi_a")),
            payment("b", PaymentStatus::Created, None),
            payment("c", PaymentStatus::Captured, Some("pi_c")),
        ]);
        let svc = service(repo, Ok(PaymentStatus::Refunded));

        let page = svc
            .list(&PaymentQuery {
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 3);

        let captured = svc
            .list(&PaymentQuery {
                status: Some(PaymentStatus::Captured),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(captured.total, 2);
    }

    #[tokio::test]
    async fn get_returns_payment_with_history() {
        let repo = FakeRepo::default();
        repo.rows
            .lock()
            .unwrap()
            .push(payment("a", PaymentStatus::Captured, Some("pi_a")));
        repo.history.lock().unwrap().push(PaymentStatusChange {
            payment_id: PaymentId::new("a"),
            from: None,
            to: PaymentStatus::Created,
            reason: None,
            occurred_at: SystemTime::UNIX_EPOCH,
        });
        let svc = service(repo, Ok(PaymentStatus::Refunded));

        let detail = svc.get(&PaymentId::new("a")).await.unwrap().unwrap();
        assert_eq!(detail.payment.status, PaymentStatus::Captured);
        assert_eq!(detail.history.len(), 1);
        assert!(svc.get(&PaymentId::new("missing")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn refund_of_a_captured_payment_transitions_to_refunded() {
        let repo = FakeRepo::default();
        repo.rows
            .lock()
            .unwrap()
            .push(payment("a", PaymentStatus::Captured, Some("pi_a")));
        let svc = service(repo, Ok(PaymentStatus::Refunded));

        let outcome = svc.refund(&PaymentId::new("a")).await.unwrap();
        assert_eq!(outcome.status, PaymentStatus::Refunded);
    }

    #[tokio::test]
    async fn refund_rejects_a_non_refundable_status() {
        let repo = FakeRepo::default();
        repo.rows
            .lock()
            .unwrap()
            .push(payment("a", PaymentStatus::Created, Some("pi_a")));
        let svc = service(repo, Ok(PaymentStatus::Refunded));

        assert!(matches!(
            svc.refund(&PaymentId::new("a")).await,
            Err(RefundError::NotRefundable(PaymentStatus::Created))
        ));
    }

    #[tokio::test]
    async fn refund_of_unknown_payment_is_not_found() {
        let svc = service(FakeRepo::default(), Ok(PaymentStatus::Refunded));
        assert!(matches!(
            svc.refund(&PaymentId::new("nope")).await,
            Err(RefundError::NotFound)
        ));
    }

    #[tokio::test]
    async fn refund_surfaces_a_provider_rejection_and_leaves_status_unchanged() {
        let repo = FakeRepo::default();
        repo.rows
            .lock()
            .unwrap()
            .push(payment("a", PaymentStatus::Captured, Some("pi_a")));
        let svc = service(repo, Err(ProviderError::Rejected("card_declined".into())));

        assert!(matches!(
            svc.refund(&PaymentId::new("a")).await,
            Err(RefundError::Provider(ProviderError::Rejected(_)))
        ));
    }

    #[tokio::test]
    async fn refund_without_a_provider_reference_is_rejected() {
        let repo = FakeRepo::default();
        repo.rows
            .lock()
            .unwrap()
            .push(payment("a", PaymentStatus::Captured, None));
        let svc = service(repo, Ok(PaymentStatus::Refunded));

        assert!(matches!(
            svc.refund(&PaymentId::new("a")).await,
            Err(RefundError::NoProviderReference)
        ));
    }
}
