//! The payment model: identity, the status state machine, and the
//! persistence port that stores both a payment and its full status history.

use std::time::SystemTime;

use crate::money::Money;

/// Our own opaque identifier for a payment — the primary key of record.
///
/// Deliberately distinct from [`ProviderReference`]: this id is stable and
/// ours from the moment a payment is created, even before any provider
/// intent exists for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaymentId(String);

impl PaymentId {
    /// Wrap an identifier string (freshly generated, or read back from
    /// storage).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PaymentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The payment provider's own reference for a payment intent/charge (e.g. a
/// Stripe `PaymentIntent` id).
///
/// Held as an opaque string — never the provider SDK's own type — which is
/// exactly what keeps provider types from crossing into this crate's public
/// API. A payment has no reference until its first [`crate::PaymentProvider`]
/// call returns one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderReference(String);

impl ProviderReference {
    /// Wrap a raw provider reference.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The raw reference, to send back to the provider or persist.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The payment state machine.
///
/// A payment moves through these states in one direction — [`Self::can_transition_to`]
/// is the single source of truth for which moves are legal, so both the
/// application layer and the storage adapter can reject an illegal move (e.g.
/// capturing an already-refunded payment) before anything is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaymentStatus {
    /// Just created in our storage; no provider intent exists yet.
    Created,
    /// The provider requires further customer action (e.g. 3-D Secure)
    /// before funds can be authorized.
    RequiresAction,
    /// Funds are reserved with the provider but not yet captured.
    Authorized,
    /// Funds have been captured (moved) in full.
    Captured,
    /// Some, but not all, of a captured amount has been returned.
    PartiallyRefunded,
    /// The full captured amount has been returned.
    Refunded,
    /// The provider declined or failed the payment; terminal.
    Failed,
    /// The payment was canceled before capture; terminal.
    Canceled,
}

impl PaymentStatus {
    /// Whether moving from `self` to `next` is a legal transition.
    ///
    /// `Failed`, `Canceled`, and `Refunded` are terminal: nothing follows
    /// them. `PartiallyRefunded` may repeat (further partial refunds) or
    /// complete into `Refunded`.
    pub fn can_transition_to(self, next: PaymentStatus) -> bool {
        use PaymentStatus::*;
        matches!(
            (self, next),
            (Created, RequiresAction | Authorized | Failed | Canceled)
                | (RequiresAction, Authorized | Failed | Canceled)
                | (Authorized, Captured | Failed | Canceled)
                | (Captured, PartiallyRefunded | Refunded)
                | (PartiallyRefunded, PartiallyRefunded | Refunded)
        )
    }

    /// The stable, lower-case-free string form persisted to storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::RequiresAction => "requires_action",
            Self::Authorized => "authorized",
            Self::Captured => "captured",
            Self::PartiallyRefunded => "partially_refunded",
            Self::Refunded => "refunded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Parse the string form written by [`Self::as_str`].
    pub fn parse(raw: &str) -> Result<Self, PaymentStatusError> {
        Ok(match raw {
            "created" => Self::Created,
            "requires_action" => Self::RequiresAction,
            "authorized" => Self::Authorized,
            "captured" => Self::Captured,
            "partially_refunded" => Self::PartiallyRefunded,
            "refunded" => Self::Refunded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            other => return Err(PaymentStatusError::Unknown(other.to_string())),
        })
    }
}

impl std::fmt::Display for PaymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stored status string did not match any known [`PaymentStatus`] — a
/// data-integrity fault, since every write goes through [`PaymentStatus::as_str`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentStatusError {
    /// The raw value read back from storage.
    Unknown(String),
}

impl std::fmt::Display for PaymentStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(raw) => write!(f, "unknown payment status {raw:?}"),
        }
    }
}

impl std::error::Error for PaymentStatusError {}

/// A payment, as the repository hands it to a caller.
#[derive(Debug, Clone)]
pub struct Payment {
    /// Our own identifier (the row's primary key).
    pub id: PaymentId,
    /// The provider's reference, once one exists.
    pub provider_reference: Option<ProviderReference>,
    /// The amount this payment is for.
    pub amount: Money,
    /// The payment's current state.
    pub status: PaymentStatus,
    /// When the payment was first created.
    pub created_at: SystemTime,
    /// When the payment's status last changed.
    pub updated_at: SystemTime,
}

/// One recorded status change — the payment's audit trail.
#[derive(Debug, Clone)]
pub struct PaymentStatusChange {
    /// The payment this change belongs to.
    pub payment_id: PaymentId,
    /// The prior status, or `None` for the row recording creation.
    pub from: Option<PaymentStatus>,
    /// The status moved to.
    pub to: PaymentStatus,
    /// An optional human-readable reason (a decline code, a refund reason, ...).
    pub reason: Option<String>,
    /// When the transition happened.
    pub occurred_at: SystemTime,
}

/// A new payment to persist, before it has a provider reference.
#[derive(Debug, Clone)]
pub struct NewPayment {
    /// The amount this payment is for.
    pub amount: Money,
    /// When the payment was created.
    pub created_at: SystemTime,
}

/// Port: persistence for payments and their status history.
///
/// Implemented by a Postgres adapter in `infrastructure`. [`Self::transition`]
/// is the only way a payment's status ever changes, and it is atomic:
/// updating the stored row and appending the history entry happen together,
/// guarded by the caller's expected current status so two concurrent
/// transitions cannot both win.
#[async_trait::async_trait]
pub trait PaymentRepository: Send + Sync {
    /// Persist a freshly created payment, returning its assigned id.
    async fn insert(&self, payment: &NewPayment) -> Result<PaymentId, PaymentRepositoryError>;

    /// Look a payment up by id, if it exists.
    async fn find(&self, id: &PaymentId) -> Result<Option<Payment>, PaymentRepositoryError>;

    /// Look a payment up by the provider's reference, if it exists. Used by the
    /// webhook handler to map a provider event back to the local payment.
    async fn find_by_provider_reference(
        &self,
        reference: &ProviderReference,
    ) -> Result<Option<Payment>, PaymentRepositoryError>;

    /// Attach the provider's reference once the provider has created its
    /// intent for this payment.
    async fn set_provider_reference(
        &self,
        id: &PaymentId,
        reference: &ProviderReference,
    ) -> Result<(), PaymentRepositoryError>;

    /// Atomically move a payment from `expected_current` to `next`, appending
    /// a history row. Fails with [`PaymentRepositoryError::Conflict`] if the
    /// payment's stored status is not `expected_current` when the write is
    /// attempted — the caller re-reads and decides whether to retry.
    #[allow(clippy::too_many_arguments)]
    async fn transition(
        &self,
        id: &PaymentId,
        expected_current: PaymentStatus,
        next: PaymentStatus,
        reason: Option<&str>,
        occurred_at: SystemTime,
    ) -> Result<(), PaymentRepositoryError>;

    /// The full, ordered status history for a payment.
    async fn history(
        &self,
        id: &PaymentId,
    ) -> Result<Vec<PaymentStatusChange>, PaymentRepositoryError>;
}

/// A storage failure from the [`PaymentRepository`] port.
#[derive(Debug)]
pub enum PaymentRepositoryError {
    /// The payment's stored status was not the caller's expected current
    /// status when [`PaymentRepository::transition`] attempted the write —
    /// either it does not exist, or another transition already moved it.
    Conflict,
    /// Any other backend failure, described for logs.
    Backend(String),
}

impl std::fmt::Display for PaymentRepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => f.write_str("payment status changed concurrently"),
            Self::Backend(msg) => write!(f, "payment repository backend error: {msg}"),
        }
    }
}

impl std::error::Error for PaymentRepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use PaymentStatus::*;

    #[test]
    fn legal_transitions_from_created() {
        assert!(Created.can_transition_to(RequiresAction));
        assert!(Created.can_transition_to(Authorized));
        assert!(Created.can_transition_to(Failed));
        assert!(Created.can_transition_to(Canceled));
        assert!(!Created.can_transition_to(Captured));
        assert!(!Created.can_transition_to(Refunded));
    }

    #[test]
    fn authorized_can_only_capture_fail_or_cancel() {
        assert!(Authorized.can_transition_to(Captured));
        assert!(Authorized.can_transition_to(Failed));
        assert!(Authorized.can_transition_to(Canceled));
        assert!(!Authorized.can_transition_to(RequiresAction));
        assert!(!Authorized.can_transition_to(Refunded));
    }

    #[test]
    fn captured_can_only_refund() {
        assert!(Captured.can_transition_to(PartiallyRefunded));
        assert!(Captured.can_transition_to(Refunded));
        assert!(!Captured.can_transition_to(Authorized));
        assert!(!Captured.can_transition_to(Failed));
    }

    #[test]
    fn partially_refunded_can_repeat_or_complete() {
        assert!(PartiallyRefunded.can_transition_to(PartiallyRefunded));
        assert!(PartiallyRefunded.can_transition_to(Refunded));
        assert!(!PartiallyRefunded.can_transition_to(Captured));
    }

    #[test]
    fn terminal_states_accept_no_further_transitions() {
        for terminal in [Refunded, Failed, Canceled] {
            for next in [
                Created,
                RequiresAction,
                Authorized,
                Captured,
                PartiallyRefunded,
                Refunded,
                Failed,
                Canceled,
            ] {
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal:?} must not transition to {next:?}"
                );
            }
        }
    }

    #[test]
    fn status_round_trips_through_its_string_form() {
        for status in [
            Created,
            RequiresAction,
            Authorized,
            Captured,
            PartiallyRefunded,
            Refunded,
            Failed,
            Canceled,
        ] {
            assert_eq!(PaymentStatus::parse(status.as_str()).unwrap(), status);
        }
    }

    #[test]
    fn unknown_status_string_is_rejected() {
        assert!(matches!(
            PaymentStatus::parse("not_a_status"),
            Err(PaymentStatusError::Unknown(_))
        ));
    }
}
