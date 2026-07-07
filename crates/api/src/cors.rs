//! CORS as an explicit origin allowlist — never a wildcard, even implicitly.
//!
//! Bead authapp-3e68cc. Static per-route headers (HSTS, frame-ancestors,
//! CSP, ...) are Traefik's job (`infra/traefik/dynamic/middlewares.yml`);
//! CORS needs per-request `Origin` matching against a caller-supplied
//! allowlist plus preflight (`OPTIONS`) handling, which is what this layer
//! does.

use axum::http::{HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::session::CSRF_HEADER;

/// Build the CORS layer from an explicit list of allowed origins.
///
/// Every entry must be an exact origin (`scheme://host[:port]`, no path, no
/// trailing slash) — an entry that fails to parse as one is dropped rather
/// than silently widening the allowlist. An empty (or entirely-invalid) list
/// means no cross-origin request is ever allowed: there is no wildcard
/// fallback, satisfying "reject origins outside the allowlist" by
/// construction rather than by a runtime toggle.
pub fn layer(allowed_origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        // Cookies (the session and CSRF cookies) only work cross-origin with
        // credentials explicitly allowed — safe to pair with an explicit
        // origin list (tower-http refuses this combination with a wildcard).
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            HeaderName::from_static(CSRF_HEADER),
        ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn app(allowed_origins: &[String]) -> Router {
        Router::new()
            .route("/ping", get(|| async { "pong" }))
            .layer(layer(allowed_origins))
    }

    #[tokio::test]
    async fn allowed_origin_is_reflected() {
        let allowed = vec!["https://admin.example.com".to_string()];
        let response = app(&allowed)
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header("origin", "https://admin.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "https://admin.example.com"
        );
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-credentials")
                .unwrap(),
            "true"
        );
    }

    #[tokio::test]
    async fn origin_outside_the_allowlist_gets_no_cors_headers() {
        let allowed = vec!["https://admin.example.com".to_string()];
        let response = app(&allowed)
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header("origin", "https://evil.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The request itself still completes (CORS is enforced by the
        // browser refusing to expose the response, not by the server
        // refusing to answer) but carries no allow-origin header, so a
        // browser's fetch() in that origin will reject reading the body.
        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
    }

    #[tokio::test]
    async fn empty_allowlist_allows_no_origin() {
        let response = app(&[])
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header("origin", "https://admin.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
    }

    #[tokio::test]
    async fn preflight_for_an_allowed_origin_permits_the_csrf_header() {
        let allowed = vec!["https://admin.example.com".to_string()];
        let response = app(&allowed)
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/ping")
                    .header("origin", "https://admin.example.com")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", CSRF_HEADER)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let allow_headers = response
            .headers()
            .get("access-control-allow-headers")
            .unwrap()
            .to_str()
            .unwrap()
            .to_ascii_lowercase();
        assert!(allow_headers.contains(CSRF_HEADER));
    }
}
