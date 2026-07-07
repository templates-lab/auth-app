//! A reference [`PaymentProvider`] adapter for Stripe (test or live mode —
//! the difference is only which secret key you configure).
//!
//! Talks to Stripe's REST API through the shared [`HttpClient`] seam, so its
//! request-building and response-parsing (and its mapping of Stripe's
//! PaymentIntent status strings onto our provider-agnostic
//! [`PaymentStatus`]) are unit-tested against a fake transport — the socket to
//! Stripe is the only untested part. No Stripe SDK type ever crosses the
//! [`PaymentProvider`] boundary.

use std::sync::Arc;

use async_trait::async_trait;
use payments::{
    Money, PaymentProvider, PaymentStatus, ProviderError, ProviderIntent, ProviderReference,
};
use serde::Deserialize;

use crate::oauth_http::{HttpClient, HttpResponse};

/// Configuration for the Stripe adapter.
#[derive(Debug, Clone)]
pub struct StripeConfig {
    /// The Stripe secret key (`sk_test_...` for test mode, `sk_live_...` for
    /// live) — sent as the `Bearer` credential on every call.
    pub secret_key: String,
    /// The API base URL. Defaults to `https://api.stripe.com`; overridable so a
    /// test can point it at a local mock.
    pub api_base: String,
}

impl StripeConfig {
    /// Build a config with the default `https://api.stripe.com` base.
    pub fn new(secret_key: impl Into<String>) -> Self {
        Self {
            secret_key: secret_key.into(),
            api_base: "https://api.stripe.com".to_string(),
        }
    }
}

/// A Stripe-backed [`PaymentProvider`].
pub struct StripeProvider {
    config: StripeConfig,
    http: Arc<dyn HttpClient>,
}

impl std::fmt::Debug for StripeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StripeProvider")
            .field("api_base", &self.config.api_base)
            .finish_non_exhaustive()
    }
}

impl StripeProvider {
    /// Build the provider from its config and an HTTP transport.
    pub fn new(config: StripeConfig, http: Arc<dyn HttpClient>) -> Self {
        Self { config, http }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.config.api_base.trim_end_matches('/'), path)
    }

    /// Map a non-2xx Stripe response to a [`ProviderError`]. A 4xx is a
    /// `Rejected` (the request was understood and declined); a 5xx or transport
    /// failure is `Unavailable` (retrying may help).
    fn error(response: &HttpResponse) -> ProviderError {
        if (400..500).contains(&response.status) {
            ProviderError::Rejected(format!("stripe returned {}", response.status))
        } else {
            ProviderError::Unavailable(format!("stripe returned {}", response.status))
        }
    }
}

/// The subset of a Stripe PaymentIntent we read.
#[derive(Debug, Deserialize)]
struct PaymentIntentResponse {
    id: String,
    status: String,
    #[serde(default)]
    amount: i64,
}

/// Map a Stripe PaymentIntent status string onto our provider-agnostic status.
fn map_status(stripe: &str) -> PaymentStatus {
    match stripe {
        "requires_capture" => PaymentStatus::Authorized,
        "succeeded" => PaymentStatus::Captured,
        "canceled" => PaymentStatus::Canceled,
        "requires_action" => PaymentStatus::RequiresAction,
        // requires_payment_method / requires_confirmation / processing — the
        // intent exists but is not yet authorized.
        _ => PaymentStatus::Created,
    }
}

#[async_trait]
impl PaymentProvider for StripeProvider {
    async fn create_intent(&self, amount: Money) -> Result<ProviderIntent, ProviderError> {
        // `capture_method=manual` so funds are authorized first and captured by
        // an explicit later call, matching this crate's Authorized -> Captured
        // state machine.
        let form = vec![
            ("amount".to_string(), amount.minor_units().to_string()),
            (
                "currency".to_string(),
                amount.currency().as_str().to_ascii_lowercase(),
            ),
            ("capture_method".to_string(), "manual".to_string()),
        ];
        let response = self
            .http
            .post_form(
                &self.url("v1/payment_intents"),
                &form,
                Some(&self.config.secret_key),
            )
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        if !response.is_success() {
            return Err(Self::error(&response));
        }
        let intent = parse_intent(&response.body)?;
        Ok(ProviderIntent {
            reference: ProviderReference::new(intent.id),
            status: map_status(&intent.status),
        })
    }

    async fn capture(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        let form = vec![(
            "amount_to_capture".to_string(),
            amount.minor_units().to_string(),
        )];
        let response = self
            .http
            .post_form(
                &self.url(&format!(
                    "v1/payment_intents/{}/capture",
                    reference.as_str()
                )),
                &form,
                Some(&self.config.secret_key),
            )
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        if !response.is_success() {
            return Err(Self::error(&response));
        }
        let intent = parse_intent(&response.body)?;
        Ok(ProviderIntent {
            reference: ProviderReference::new(intent.id),
            status: map_status(&intent.status),
        })
    }

    async fn refund(
        &self,
        reference: &ProviderReference,
        amount: Money,
    ) -> Result<ProviderIntent, ProviderError> {
        // Create the refund.
        let form = vec![
            ("payment_intent".to_string(), reference.as_str().to_string()),
            ("amount".to_string(), amount.minor_units().to_string()),
        ];
        let response = self
            .http
            .post_form(
                &self.url("v1/refunds"),
                &form,
                Some(&self.config.secret_key),
            )
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        if !response.is_success() {
            return Err(Self::error(&response));
        }

        // Stripe keeps the PaymentIntent `succeeded` after a refund (refund
        // totals live on the charge), so we decide full-vs-partial here by
        // comparing this refund's amount to the intent's total. Cumulative
        // refund accounting across multiple refunds is the application layer's
        // job — it owns the payment history table.
        let intent = self.fetch_intent(reference).await?;
        let status = if amount.minor_units() >= intent.amount {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        Ok(ProviderIntent {
            reference: reference.clone(),
            status,
        })
    }

    async fn get_status(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentStatus, ProviderError> {
        let intent = self.fetch_intent(reference).await?;
        Ok(map_status(&intent.status))
    }
}

impl StripeProvider {
    async fn fetch_intent(
        &self,
        reference: &ProviderReference,
    ) -> Result<PaymentIntentResponse, ProviderError> {
        let response = self
            .http
            .get_bearer(
                &self.url(&format!("v1/payment_intents/{}", reference.as_str())),
                &self.config.secret_key,
            )
            .await
            .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        if !response.is_success() {
            return Err(Self::error(&response));
        }
        parse_intent(&response.body)
    }
}

fn parse_intent(body: &str) -> Result<PaymentIntentResponse, ProviderError> {
    serde_json::from_str(body)
        .map_err(|e| ProviderError::Unavailable(format!("stripe response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::oauth_http::HttpError;
    use payments::Currency;

    /// One recorded POST: url, form fields, and the bearer sent.
    type RecordedPost = (String, Vec<(String, String)>, Option<String>);

    /// A transport that replays a queue of canned responses in order, and
    /// records every request for assertions.
    struct ScriptedHttp {
        responses: Mutex<VecDeque<HttpResponse>>,
        posts: Mutex<Vec<RecordedPost>>,
        gets: Mutex<Vec<String>>,
    }

    impl ScriptedHttp {
        fn new(responses: Vec<HttpResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
                posts: Mutex::new(Vec::new()),
                gets: Mutex::new(Vec::new()),
            })
        }
        fn next(&self) -> HttpResponse {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("a scripted response for every call")
        }
    }

    #[async_trait]
    impl HttpClient for ScriptedHttp {
        async fn post_form(
            &self,
            url: &str,
            form: &[(String, String)],
            bearer: Option<&str>,
        ) -> Result<HttpResponse, HttpError> {
            self.posts.lock().unwrap().push((
                url.to_string(),
                form.to_vec(),
                bearer.map(str::to_string),
            ));
            Ok(self.next())
        }
        async fn get_bearer(&self, url: &str, _bearer: &str) -> Result<HttpResponse, HttpError> {
            self.gets.lock().unwrap().push(url.to_string());
            Ok(self.next())
        }
    }

    fn ok(body: serde_json::Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_string(),
        }
    }

    fn usd(minor: i64) -> Money {
        Money::from_minor_units(minor, Currency::parse("USD").unwrap()).unwrap()
    }

    fn provider(http: Arc<ScriptedHttp>) -> StripeProvider {
        StripeProvider::new(StripeConfig::new("sk_test_123"), http)
    }

    #[tokio::test]
    async fn create_intent_sends_manual_capture_and_bearer_and_maps_status() {
        let http = ScriptedHttp::new(vec![ok(serde_json::json!({
            "id": "pi_123", "status": "requires_payment_method", "amount": 2500
        }))]);
        let intent = provider(http.clone())
            .create_intent(usd(2500))
            .await
            .unwrap();

        assert_eq!(intent.reference.as_str(), "pi_123");
        assert_eq!(intent.status, PaymentStatus::Created);

        let posts = http.posts.lock().unwrap();
        let (url, form, bearer) = &posts[0];
        assert!(url.ends_with("/v1/payment_intents"));
        assert_eq!(bearer.as_deref(), Some("sk_test_123"));
        assert!(form
            .iter()
            .any(|(k, v)| k == "capture_method" && v == "manual"));
        assert!(form.iter().any(|(k, v)| k == "amount" && v == "2500"));
        assert!(form.iter().any(|(k, v)| k == "currency" && v == "usd"));
    }

    #[tokio::test]
    async fn capture_maps_succeeded_to_captured() {
        let http = ScriptedHttp::new(vec![ok(serde_json::json!({
            "id": "pi_123", "status": "succeeded", "amount": 2500
        }))]);
        let intent = provider(http.clone())
            .capture(&ProviderReference::new("pi_123"), usd(2500))
            .await
            .unwrap();
        assert_eq!(intent.status, PaymentStatus::Captured);
        assert!(http.posts.lock().unwrap()[0]
            .0
            .ends_with("/v1/payment_intents/pi_123/capture"));
    }

    #[tokio::test]
    async fn full_refund_maps_to_refunded() {
        // First response: the refund object; second: the fetched intent.
        let http = ScriptedHttp::new(vec![
            ok(serde_json::json!({"id": "re_1", "status": "succeeded"})),
            ok(serde_json::json!({"id": "pi_123", "status": "succeeded", "amount": 2500})),
        ]);
        let intent = provider(http.clone())
            .refund(&ProviderReference::new("pi_123"), usd(2500))
            .await
            .unwrap();
        assert_eq!(intent.status, PaymentStatus::Refunded);
    }

    #[tokio::test]
    async fn partial_refund_maps_to_partially_refunded() {
        let http = ScriptedHttp::new(vec![
            ok(serde_json::json!({"id": "re_1", "status": "succeeded"})),
            ok(serde_json::json!({"id": "pi_123", "status": "succeeded", "amount": 2500})),
        ]);
        let intent = provider(http.clone())
            .refund(&ProviderReference::new("pi_123"), usd(1000))
            .await
            .unwrap();
        assert_eq!(intent.status, PaymentStatus::PartiallyRefunded);
    }

    #[tokio::test]
    async fn get_status_maps_requires_capture_to_authorized() {
        let http = ScriptedHttp::new(vec![ok(serde_json::json!({
            "id": "pi_123", "status": "requires_capture", "amount": 2500
        }))]);
        let status = provider(http)
            .get_status(&ProviderReference::new("pi_123"))
            .await
            .unwrap();
        assert_eq!(status, PaymentStatus::Authorized);
    }

    #[tokio::test]
    async fn full_intent_capture_refund_flow() {
        // The AC's end-to-end path (intento → captura → reembolso), each step
        // driven through the real adapter against a Stripe-shaped response.
        let http = ScriptedHttp::new(vec![
            ok(
                serde_json::json!({"id": "pi_9", "status": "requires_payment_method", "amount": 5000}),
            ),
            ok(serde_json::json!({"id": "pi_9", "status": "succeeded", "amount": 5000})),
            // refund object, then the fetched intent for full-vs-partial.
            ok(serde_json::json!({"id": "re_9", "status": "succeeded"})),
            ok(serde_json::json!({"id": "pi_9", "status": "succeeded", "amount": 5000})),
        ]);
        let provider = provider(http);
        let intent = provider.create_intent(usd(5000)).await.unwrap();
        assert_eq!(intent.status, PaymentStatus::Created);

        let captured = provider
            .capture(&intent.reference, usd(5000))
            .await
            .unwrap();
        assert_eq!(captured.status, PaymentStatus::Captured);

        let refunded = provider.refund(&intent.reference, usd(5000)).await.unwrap();
        assert_eq!(refunded.status, PaymentStatus::Refunded);
    }

    #[tokio::test]
    async fn a_4xx_is_rejected_and_a_5xx_is_unavailable() {
        let declined = ScriptedHttp::new(vec![HttpResponse {
            status: 402,
            body: "{}".to_string(),
        }]);
        assert!(matches!(
            provider(declined).create_intent(usd(2500)).await,
            Err(ProviderError::Rejected(_))
        ));

        let down = ScriptedHttp::new(vec![HttpResponse {
            status: 503,
            body: "{}".to_string(),
        }]);
        assert!(matches!(
            provider(down).create_intent(usd(2500)).await,
            Err(ProviderError::Unavailable(_))
        ));
    }
}
