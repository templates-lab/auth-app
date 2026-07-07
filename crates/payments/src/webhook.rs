//! Payment webhooks: the provider-agnostic model and ports for receiving,
//! verifying, deduplicating, and persisting a provider's asynchronous status
//! notifications.
//!
//! Like the rest of this crate, everything here is pure: a distilled
//! [`WebhookEvent`], the [`WebhookVerifier`] a provider adapter implements to
//! authenticate and parse a raw payload, and the [`WebhookEventStore`] a
//! Postgres adapter implements for the raw-event log and idempotency. No HTTP,
//! no provider SDK, no database driver.

use crate::{PaymentStatus, ProviderReference};

/// A provider webhook, distilled to what we act on: its unique event id (for
/// idempotency), the payment it concerns, and the status it reports.
///
/// `reference`/`new_status` are optional because a valid, correctly-signed
/// event may not map to a payment status change we act on (an event type we
/// don't handle) — such an event is still logged and deduplicated, just a
/// no-op to process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookEvent {
    /// The provider's globally-unique event id — the idempotency key.
    pub event_id: String,
    /// The provider reference of the payment this event concerns, if any.
    pub reference: Option<ProviderReference>,
    /// The status this event reports for that payment, if it maps to one.
    pub new_status: Option<PaymentStatus>,
}

/// Why verifying/parsing a raw webhook failed.
#[derive(Debug)]
pub enum WebhookError {
    /// The signature was missing or did not verify — the payload is not
    /// trustworthy and must be rejected (and audited) without being acted on.
    InvalidSignature,
    /// The signature verified but the payload could not be parsed into an
    /// event (a provider/schema mismatch).
    Malformed(String),
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => f.write_str("webhook signature is missing or invalid"),
            Self::Malformed(m) => write!(f, "webhook payload is malformed: {m}"),
        }
    }
}

impl std::error::Error for WebhookError {}

/// Port: authenticates and parses a provider's raw webhook payload.
///
/// Implemented per provider (a Stripe adapter, ...). The signature is verified
/// over the *raw* bytes — exactly as received — because any re-serialization
/// would change them and break the HMAC.
pub trait WebhookVerifier: Send + Sync {
    /// Verify `signature` over `payload` and distill the event. `signature` is
    /// whatever the provider's signature header carried (absent ⇒
    /// [`WebhookError::InvalidSignature`]).
    fn verify(&self, payload: &[u8], signature: Option<&str>)
        -> Result<WebhookEvent, WebhookError>;
}

/// Port: the raw-event log and idempotency store.
///
/// Backed by a Postgres table (`payment_webhook_events`) that both keeps every
/// received payload for diagnostics/replay and enforces once-only processing
/// by event id.
#[async_trait::async_trait]
pub trait WebhookEventStore: Send + Sync {
    /// Persist a *rejected* receipt (bad or missing signature, or malformed) —
    /// the audit/diagnostic record for a webhook that will not be acted on.
    /// `reason` is a short machine label (e.g. `invalid_signature`).
    async fn record_rejected(
        &self,
        payload: &[u8],
        signature: Option<&str>,
        reason: &str,
    ) -> Result<(), WebhookStoreError>;

    /// Persist a verified receipt and atomically claim its `event_id` for
    /// processing. Returns `true` if this is the first time the event id is
    /// seen (the caller should process it) or `false` if it was already
    /// recorded (a duplicate — the caller must not act again). Either way the
    /// raw payload is persisted.
    async fn record_and_claim(
        &self,
        event_id: &str,
        payload: &[u8],
    ) -> Result<bool, WebhookStoreError>;
}

/// A storage failure from the [`WebhookEventStore`] port.
#[derive(Debug)]
pub struct WebhookStoreError(pub String);

impl std::fmt::Display for WebhookStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "webhook store error: {}", self.0)
    }
}

impl std::error::Error for WebhookStoreError {}
