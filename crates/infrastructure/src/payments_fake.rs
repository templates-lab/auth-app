//! A deterministic in-memory [`PaymentProvider`] for local development and
//! integration tests — no credentials, no network.
//!
//! It is a real, env-selectable adapter (not just a test double): pointing the
//! server at it (`PAYMENT_PROVIDER=fake`) lets the whole payment flow run
//! locally. Outcomes are driven by the amount's cents so a caller can force
//! each branch deterministically, the same idea as a provider's test cards:
//!
//! - cents `== 1` (e.g. `$X.01`) → [`ProviderError::Rejected`] (a decline)
//! - cents `== 2` (e.g. `$X.02`) → [`ProviderError::Unavailable`] (a timeout)
//! - anything else → success
//!
//! Created intents are tracked in memory, so `capture`, `refund`, and
//! `get_status` stay coherent with what was created.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use payments::{
    Money, PaymentProvider, PaymentStatus, ProviderError, ProviderIntent, ProviderReference,
};

/// A tracked intent: its authorized amount and current status.
#[derive(Debug, Clone)]
struct Intent {
    amount: Money,
    status: PaymentStatus,
}

/// A deterministic, in-memory payment provider.
#[derive(Debug, Default)]
pub struct FakePaymentProvider {
    intents: Mutex<HashMap<String, Intent>>,
    counter: AtomicU64,
}

impl FakePaymentProvider {
    /// A fresh provider with no intents.
    pub fn new() -> Self {
        Self::default()
    }

    /// The outcome the amount's cents force, if any.
    fn forced_error(amount: Money) -> Option<ProviderError> {
        match amount.minor_units() % 100 {
            1 => Some(ProviderError::Rejected("card_declined (simulated)".into())),
            2 => Some(ProviderError::Unavailable("timeout (simulated)".into())),
            _ => None,
        }
    }

    fn get(&self, reference: &ProviderReference) -> Result<Intent, ProviderError> {
        self.intents
            .lock()
            .unwrap()
            .get(reference.as_str())
            .cloned()
            .ok_or_else(|| ProviderError::Rejected("unknown payment reference".into()))
    }
}

#[async_trait]
impl PaymentProvider for FakePaymentProvider {
    async fn create_intent(&self, amount: Money) -> Result<ProviderIntent, ProviderError> {
        if let Some(err) = Self::forced_error(amount) {
            return Err(err);
        }
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let reference = format!("fake_pi_{n}");
        self.intents.lock().unwrap().insert(
            reference.clone(),
            Intent {
                amount,
                status: PaymentStatus::Created,
            },
        );
        Ok(ProviderIntent {
            reference: ProviderReference::new(reference),
            status: PaymentStatus::Created,
        })
    }

    async fn capture(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        if let Some(err) = Self::forced_error(amount) {
            return Err(err);
        }
        let mut intents = self.intents.lock().unwrap();
        let intent = intents
            .get_mut(reference.as_str())
            .ok_or_else(|| ProviderError::Rejected("unknown payment reference".into()))?;
        intent.status = PaymentStatus::Captured;
        Ok(ProviderIntent {
            reference: reference.clone(),
            status: PaymentStatus::Captured,
        })
    }

    async fn refund(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        if let Some(err) = Self::forced_error(amount) {
            return Err(err);
        }
        let mut intents = self.intents.lock().unwrap();
        let intent = intents
            .get_mut(reference.as_str())
            .ok_or_else(|| ProviderError::Rejected("unknown payment reference".into()))?;
        // A refund covering the full authorized amount is a full refund.
        intent.status = if amount.minor_units() >= intent.amount.minor_units() {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        Ok(ProviderIntent {
            reference: reference.clone(),
            status: intent.status,
        })
    }

    async fn get_status(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentStatus, ProviderError> {
        Ok(self.get(reference)?.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use payments::Currency;

    fn usd(minor: i64) -> Money {
        Money::from_minor_units(minor, Currency::parse("USD").unwrap()).unwrap()
    }

    #[tokio::test]
    async fn create_capture_refund_flow_is_coherent() {
        let p = FakePaymentProvider::new();
        let created = p.create_intent(usd(2500)).await.unwrap();
        assert_eq!(created.status, PaymentStatus::Created);

        let captured = p.capture(&created.reference, usd(2500)).await.unwrap();
        assert_eq!(captured.status, PaymentStatus::Captured);
        assert_eq!(
            p.get_status(&created.reference).await.unwrap(),
            PaymentStatus::Captured
        );

        let refunded = p.refund(&created.reference, usd(2500)).await.unwrap();
        assert_eq!(refunded.status, PaymentStatus::Refunded);
    }

    #[tokio::test]
    async fn partial_refund_is_partially_refunded() {
        let p = FakePaymentProvider::new();
        let created = p.create_intent(usd(2500)).await.unwrap();
        p.capture(&created.reference, usd(2500)).await.unwrap();
        let refunded = p.refund(&created.reference, usd(1000)).await.unwrap();
        assert_eq!(refunded.status, PaymentStatus::PartiallyRefunded);
    }

    #[tokio::test]
    async fn cents_01_forces_a_decline_and_cents_02_a_timeout() {
        let p = FakePaymentProvider::new();
        assert!(matches!(
            p.create_intent(usd(2501)).await,
            Err(ProviderError::Rejected(_))
        ));
        assert!(matches!(
            p.create_intent(usd(2502)).await,
            Err(ProviderError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn unknown_reference_is_rejected() {
        let p = FakePaymentProvider::new();
        assert!(matches!(
            p.get_status(&ProviderReference::new("nope")).await,
            Err(ProviderError::Rejected(_))
        ));
    }
}
