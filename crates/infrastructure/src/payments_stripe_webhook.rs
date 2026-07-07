//! Stripe implementation of the [`WebhookVerifier`] port.
//!
//! Verifies the `Stripe-Signature` header (an HMAC-SHA256 over
//! `"{timestamp}.{raw_body}"`, keyed by the endpoint's signing secret) in
//! constant time, rejects a timestamp outside the tolerance window (replay
//! protection), then distils the event into a provider-agnostic
//! [`WebhookEvent`]. The signature is checked over the *raw* bytes — never a
//! re-serialization, which would change them.

use hmac::{Hmac, Mac};
use payments::{PaymentStatus, ProviderReference, WebhookError, WebhookEvent, WebhookVerifier};
use serde::Deserialize;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// How far a signature's timestamp may be from now before it is rejected as a
/// possible replay (Stripe's own default is 5 minutes).
const DEFAULT_TOLERANCE_SECS: u64 = 300;

/// Configuration for the Stripe webhook verifier.
#[derive(Debug, Clone)]
pub struct StripeWebhookConfig {
    /// The endpoint's signing secret (`whsec_...`).
    pub signing_secret: String,
    /// The allowed clock skew, in seconds.
    pub tolerance_secs: u64,
}

impl StripeWebhookConfig {
    /// Build a config with the default 5-minute tolerance.
    pub fn new(signing_secret: impl Into<String>) -> Self {
        Self {
            signing_secret: signing_secret.into(),
            tolerance_secs: DEFAULT_TOLERANCE_SECS,
        }
    }
}

/// A Stripe [`WebhookVerifier`].
#[derive(Debug, Clone)]
pub struct StripeWebhookVerifier {
    config: StripeWebhookConfig,
}

impl StripeWebhookVerifier {
    /// Build the verifier from its config.
    pub fn new(config: StripeWebhookConfig) -> Self {
        Self { config }
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// The minimal Stripe event envelope we read.
#[derive(Debug, Deserialize)]
struct StripeEvent {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    data: StripeData,
}

#[derive(Debug, Deserialize)]
struct StripeData {
    object: StripeObject,
}

#[derive(Debug, Deserialize)]
struct StripeObject {
    #[serde(default)]
    id: Option<String>,
    /// Present on charge/refund objects — the id of the related PaymentIntent.
    #[serde(default)]
    payment_intent: Option<String>,
}

impl WebhookVerifier for StripeWebhookVerifier {
    fn verify(
        &self,
        payload: &[u8],
        signature: Option<&str>,
    ) -> Result<WebhookEvent, WebhookError> {
        let header = signature.ok_or(WebhookError::InvalidSignature)?;
        let (timestamp, v1) =
            parse_signature_header(header).ok_or(WebhookError::InvalidSignature)?;

        // Replay protection: reject a timestamp outside the tolerance window.
        let now = Self::now_unix();
        let skew = now.abs_diff(timestamp);
        if skew > self.config.tolerance_secs {
            return Err(WebhookError::InvalidSignature);
        }

        // HMAC over exactly `"{t}.{raw_body}"`.
        let mut mac = HmacSha256::new_from_slice(self.config.signing_secret.as_bytes())
            .map_err(|_| WebhookError::InvalidSignature)?;
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        let expected = hex_decode(&v1).ok_or(WebhookError::InvalidSignature)?;
        // Constant-time comparison via the MAC's own verify.
        mac.verify_slice(&expected)
            .map_err(|_| WebhookError::InvalidSignature)?;

        // Signature good — now parse the event body.
        let event: StripeEvent =
            serde_json::from_slice(payload).map_err(|e| WebhookError::Malformed(e.to_string()))?;
        Ok(distill(event))
    }
}

/// Map a Stripe event onto the provider-agnostic [`WebhookEvent`].
fn distill(event: StripeEvent) -> WebhookEvent {
    let (reference, new_status) = match event.event_type.as_str() {
        "payment_intent.amount_capturable_updated" => {
            (event.data.object.id, Some(PaymentStatus::Authorized))
        }
        "payment_intent.succeeded" => (event.data.object.id, Some(PaymentStatus::Captured)),
        "payment_intent.payment_failed" => (event.data.object.id, Some(PaymentStatus::Failed)),
        "payment_intent.canceled" => (event.data.object.id, Some(PaymentStatus::Canceled)),
        "payment_intent.requires_action" => {
            (event.data.object.id, Some(PaymentStatus::RequiresAction))
        }
        // A refund event's object is a charge; the payment it belongs to is in
        // `payment_intent`. Full-vs-partial accounting is the app layer's job,
        // so we report the terminal Refunded and let the state machine guard
        // reject it if only a partial move is legal.
        "charge.refunded" => (
            event.data.object.payment_intent,
            Some(PaymentStatus::Refunded),
        ),
        // A valid event we take no action on: logged and deduplicated, no-op.
        _ => (None, None),
    };
    WebhookEvent {
        event_id: event.id,
        reference: reference.map(ProviderReference::new),
        new_status,
    }
}

/// Parse a `Stripe-Signature` header (`t=...,v1=...,v1=...`) into the timestamp
/// and the first `v1` scheme signature.
fn parse_signature_header(header: &str) -> Option<(u64, String)> {
    let mut timestamp = None;
    let mut v1 = None;
    for part in header.split(',') {
        let (k, value) = part.split_once('=')?;
        match k.trim() {
            "t" => timestamp = value.trim().parse::<u64>().ok(),
            "v1" if v1.is_none() => v1 = Some(value.trim().to_string()),
            _ => {}
        }
    }
    Some((timestamp?, v1?))
}

/// Decode a lowercase/uppercase hex string to bytes; `None` on any non-hex
/// input or odd length.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret";

    /// Build a valid `Stripe-Signature` header for `body` at the current time.
    fn sign(body: &[u8]) -> String {
        let t = StripeWebhookVerifier::now_unix();
        let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        let sig = mac.finalize().into_bytes();
        let hex: String = sig.iter().map(|b| format!("{b:02x}")).collect();
        format!("t={t},v1={hex}")
    }

    fn verifier() -> StripeWebhookVerifier {
        StripeWebhookVerifier::new(StripeWebhookConfig::new(SECRET))
    }

    fn body(event_type: &str, intent_id: &str) -> Vec<u8> {
        serde_json::json!({
            "id": "evt_123",
            "type": event_type,
            "data": {"object": {"id": intent_id}}
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_valid_signature_parses_a_succeeded_event() {
        let payload = body("payment_intent.succeeded", "pi_1");
        let sig = sign(&payload);
        let event = verifier().verify(&payload, Some(&sig)).unwrap();
        assert_eq!(event.event_id, "evt_123");
        assert_eq!(event.reference.unwrap().as_str(), "pi_1");
        assert_eq!(event.new_status, Some(PaymentStatus::Captured));
    }

    #[test]
    fn a_tampered_body_fails_verification() {
        let payload = body("payment_intent.succeeded", "pi_1");
        let sig = sign(&payload);
        // Verify a different body against a signature for the original.
        let tampered = body("payment_intent.succeeded", "pi_ATTACKER");
        assert!(matches!(
            verifier().verify(&tampered, Some(&sig)),
            Err(WebhookError::InvalidSignature)
        ));
    }

    #[test]
    fn a_missing_signature_is_invalid() {
        let payload = body("payment_intent.succeeded", "pi_1");
        assert!(matches!(
            verifier().verify(&payload, None),
            Err(WebhookError::InvalidSignature)
        ));
    }

    #[test]
    fn a_wrong_secret_is_invalid() {
        let payload = body("payment_intent.succeeded", "pi_1");
        let sig = sign(&payload);
        let other = StripeWebhookVerifier::new(StripeWebhookConfig::new("whsec_other"));
        assert!(matches!(
            other.verify(&payload, Some(&sig)),
            Err(WebhookError::InvalidSignature)
        ));
    }

    #[test]
    fn an_expired_timestamp_is_invalid() {
        let payload = body("payment_intent.succeeded", "pi_1");
        // Sign with a timestamp far in the past.
        let t = StripeWebhookVerifier::now_unix() - 10_000;
        let mut mac = HmacSha256::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(&payload);
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let sig = format!("t={t},v1={hex}");
        assert!(matches!(
            verifier().verify(&payload, Some(&sig)),
            Err(WebhookError::InvalidSignature)
        ));
    }

    #[test]
    fn a_refund_event_maps_to_refunded_via_payment_intent() {
        let payload = serde_json::json!({
            "id": "evt_r",
            "type": "charge.refunded",
            "data": {"object": {"id": "ch_1", "payment_intent": "pi_9"}}
        })
        .to_string()
        .into_bytes();
        let sig = sign(&payload);
        let event = verifier().verify(&payload, Some(&sig)).unwrap();
        assert_eq!(event.reference.unwrap().as_str(), "pi_9");
        assert_eq!(event.new_status, Some(PaymentStatus::Refunded));
    }

    #[test]
    fn an_unhandled_event_type_is_a_valid_no_op() {
        let payload = body("customer.created", "cus_1");
        let sig = sign(&payload);
        let event = verifier().verify(&payload, Some(&sig)).unwrap();
        assert_eq!(event.event_id, "evt_123");
        assert_eq!(event.reference, None);
        assert_eq!(event.new_status, None);
    }
}
