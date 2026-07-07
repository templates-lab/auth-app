//! Port: [`PaymentProvider`], the seam every payment gateway adapter (Stripe,
//! a fake for tests, ...) implements.

use crate::money::Money;
use crate::payment::{PaymentStatus, ProviderReference};

/// The provider's view of an intent after a mutating call: its own reference
/// and the status that call produced. Returned instead of a provider SDK type
/// so no adapter's types ever cross into this crate's public API — swapping
/// providers changes only which adapter produces this same shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIntent {
    /// The provider's own identifier for this intent/charge.
    pub reference: ProviderReference,
    /// The provider's status for the intent after this call.
    pub status: PaymentStatus,
}

/// Port: a payment gateway.
///
/// Each operation's semantics are fixed regardless of which concrete provider
/// implements it — that stability is what lets the application layer and the
/// stored state machine ([`crate::PaymentStatus`]) stay provider-agnostic.
/// Implementations MUST NOT expose any provider-SDK type through this trait;
/// everything crossing the boundary is one of this crate's own types.
#[async_trait::async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Create a new payment intent for `amount` with the provider.
    ///
    /// Returns the provider's reference for the intent and its initial
    /// status — typically [`PaymentStatus::Created`] or
    /// [`PaymentStatus::RequiresAction`] if the provider demands further
    /// customer action (e.g. 3-D Secure) before funds can be authorized.
    /// Never returns [`PaymentStatus::Captured`] directly: capturing is
    /// always a separate, explicit call.
    async fn create_intent(&self, amount: Money) -> Result<ProviderIntent, ProviderError>;

    /// Capture funds previously authorized for `reference`.
    ///
    /// `amount` may be less than the original authorization for a partial
    /// capture, where the provider supports it. Moves the intent toward
    /// [`PaymentStatus::Captured`] on success.
    async fn capture(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError>;

    /// Refund `amount` of a previously captured payment.
    ///
    /// A full refund moves the intent to [`PaymentStatus::Refunded`]; a
    /// partial refund to [`PaymentStatus::PartiallyRefunded`].
    async fn refund(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError>;

    /// Fetch the provider's current status for `reference`.
    ///
    /// Used to reconcile our stored state against the provider's — for
    /// example after a webhook was missed or arrived out of order.
    async fn get_status(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentStatus, ProviderError>;
}

/// Why a [`PaymentProvider`] call failed.
///
/// Deliberately coarse and provider-agnostic: a concrete adapter maps its
/// SDK's rich error taxonomy down to one of these two buckets rather than
/// leaking it through this trait.
#[derive(Debug)]
pub enum ProviderError {
    /// The provider understood the request and declined or rejected it (bad
    /// parameters, a declined card, an amount exceeding what remains
    /// available to capture/refund, ...). Retrying the identical request is
    /// not expected to succeed.
    Rejected(String),
    /// The provider could not be reached, or returned a failure that carries
    /// no more specific meaning to us (timeout, 5xx, malformed response).
    /// Retrying may succeed.
    Unavailable(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(msg) => write!(f, "payment provider rejected the request: {msg}"),
            Self::Unavailable(msg) => write!(f, "payment provider unavailable: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}
