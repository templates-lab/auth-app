//! Request-scoped observability (bead authapp-e2bac5).
//!
//! Every request is assigned an id that is threaded through three places at
//! once: the `tracing` span it runs in (so every structured log line emitted
//! during the request carries `request_id`), the `x-request-id` response header
//! (so a client — or an upstream proxy — can correlate), and a task-local the
//! error type reads (so an [`crate::error::ApiError`]'s `trace_id` is that same
//! id). One id, visible in the logs, the response header, and the error body.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;

/// The request-id header, both accepted (to continue a trace an upstream proxy
/// started) and echoed back on the response.
const REQUEST_ID_HEADER: &str = "x-request-id";

tokio::task_local! {
    static REQUEST_ID: String;
}

/// The current request's id, when called from within a request handled by
/// [`request_context`]. Returns `None` outside that scope (e.g. in a unit test),
/// so callers fall back to [`new_id`].
pub fn current_request_id() -> Option<String> {
    REQUEST_ID.try_with(|id| id.clone()).ok()
}

/// A fresh, short correlation id. Not cryptographic — just unique enough to tie
/// a request's logs, its response header, and any error body together: the
/// current time in nanoseconds mixed with a per-process monotonic counter.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}", nanos ^ n.rotate_left(32))
}

/// Outermost middleware: assign the request an id (reusing a sane inbound
/// `x-request-id` for cross-service tracing, else generating one), run the rest
/// of the stack inside a `request` span carrying that id — and inside the
/// task-local the error type reads — then echo the id in the response header.
pub async fn request_context(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.is_ascii())
        .map(str::to_string)
        .unwrap_or_else(new_id);

    let span = tracing::info_span!("request", %method, path = %path, request_id = %id);

    // Run the stack inside the span and the request-id task-local, then emit one
    // structured completion line per request — carrying the span's `request_id`,
    // so every request produces a JSON log tied to the id the client sees.
    let served = async move {
        let response = next.run(request).await;
        tracing::info!(status = response.status().as_u16(), "request completed");
        response
    };
    let mut response = REQUEST_ID.scope(id.clone(), served.instrument(span)).await;

    if let Ok(value) = HeaderValue::from_str(&id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct() {
        assert_ne!(new_id(), new_id());
    }

    #[tokio::test]
    async fn current_request_id_is_none_outside_a_request() {
        assert!(current_request_id().is_none());
    }

    #[tokio::test]
    async fn current_request_id_is_readable_inside_the_scope() {
        let seen = REQUEST_ID
            .scope("abc123".to_string(), async { current_request_id() })
            .await;
        assert_eq!(seen.as_deref(), Some("abc123"));
    }
}
