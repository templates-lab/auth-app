//! The single error type the HTTP layer returns (bead authapp-ef053a).
//!
//! Every handler and middleware maps its failures to [`ApiError`], which
//! renders to one consistent JSON shape ([`ErrorResponse`]) — a machine `code`,
//! a client-safe `message`, a correlation `trace_id`, and, for validation
//! failures, a per-field `fields` map. This is the whole API's error contract,
//! so a client parses one shape everywhere and the generated TypeScript client
//! gets one error schema.
//!
//! Internal (`5xx`) failures never leak their cause: the detail is logged
//! server-side against the `trace_id` and the client receives only a generic
//! message plus that id, so an operator can correlate a report to a log line
//! without the client ever seeing a stack of internal strings.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// The JSON body every error response carries.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorResponse {
    /// A stable, machine-readable error code (e.g. `unauthorized`,
    /// `not_found`, `validation_failed`).
    pub code: &'static str,
    /// A human-readable, client-safe message. Never contains internal detail.
    pub message: String,
    /// A correlation id, also logged server-side, so a client-reported error
    /// can be matched to its log line.
    pub trace_id: String,
    /// Per-field validation messages, present only on a `422` response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
}

/// A typed HTTP error. Construct one with the helper for the situation and
/// return it (as `Err(..)` from a `Result`-returning handler, or via
/// [`IntoResponse`] directly in middleware); the response shape is uniform.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    fields: Option<BTreeMap<String, String>>,
    /// Internal detail logged (never sent) when this error renders. Set for
    /// `5xx` errors whose cause must not reach the client.
    internal: Option<String>,
}

impl ApiError {
    /// Build an error with an explicit status, code, and client-safe message.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            fields: None,
            internal: None,
        }
    }

    /// `401` — no valid session/credentials.
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required.",
        )
    }

    /// `403` — authenticated but not permitted (role or CSRF).
    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    /// `404` — the addressed resource does not exist.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// `400` — the request was malformed in a way with no per-field detail.
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// `409` — the request conflicts with the resource's current state.
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    /// `422` — validation failed; `fields` maps each rejected field to why.
    pub fn validation(fields: BTreeMap<String, String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "validation_failed",
            message: "The request failed validation.".to_string(),
            fields: Some(fields),
            internal: None,
        }
    }

    /// `502` — an upstream dependency (e.g. a payment provider) was unavailable.
    pub fn bad_gateway(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, code, message)
    }

    /// `500` — an internal failure. `detail` is logged against the trace id and
    /// never sent to the client, which receives only a generic message.
    pub fn internal(detail: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: "An internal error occurred.".to_string(),
            fields: None,
            internal: Some(detail.into()),
        }
    }

    /// The HTTP status this error renders with.
    pub fn status(&self) -> StatusCode {
        self.status
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let trace_id = next_trace_id();

        // Log internal detail against the trace id — the only place a 5xx cause
        // is recorded — so the client never sees it but an operator can.
        if let Some(detail) = &self.internal {
            eprintln!("api error [{trace_id}] {}: {detail}", self.code);
        }

        let body = ErrorResponse {
            code: self.code,
            message: self.message,
            trace_id,
            fields: self.fields,
        };
        (self.status, Json(body)).into_response()
    }
}

/// A short correlation id. Not cryptographic — just unique enough to tie a
/// client-visible error to its server log line: the current time in nanoseconds
/// mixed with a monotonic per-process counter, rendered as hex.
fn next_trace_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}", nanos ^ n.rotate_left(32))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn every_error_carries_code_message_and_trace_id() {
        let response = ApiError::not_found("No such widget.").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = body_json(response).await;
        assert_eq!(json["code"], "not_found");
        assert_eq!(json["message"], "No such widget.");
        assert!(json["trace_id"].as_str().is_some_and(|s| !s.is_empty()));
        // No `fields` key on a non-validation error.
        assert!(json.get("fields").is_none());
    }

    #[tokio::test]
    async fn internal_error_hides_its_cause_but_keeps_a_trace_id() {
        let response = ApiError::internal("connection refused to 10.0.0.5:5432").into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(response).await;
        assert_eq!(json["code"], "internal_error");
        assert_eq!(json["message"], "An internal error occurred.");
        // The internal detail must not appear anywhere in the client body.
        assert!(!json.to_string().contains("10.0.0.5"));
        assert!(json["trace_id"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn validation_error_is_422_with_per_field_detail() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "email".to_string(),
            "must be a valid email address".to_string(),
        );
        fields.insert("password".to_string(), "must not be empty".to_string());
        let response = ApiError::validation(fields).into_response();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let json = body_json(response).await;
        assert_eq!(json["code"], "validation_failed");
        assert_eq!(json["fields"]["email"], "must be a valid email address");
        assert_eq!(json["fields"]["password"], "must not be empty");
    }

    #[test]
    fn trace_ids_are_distinct_across_calls() {
        assert_ne!(next_trace_id(), next_trace_id());
    }
}
