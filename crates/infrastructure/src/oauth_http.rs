//! The HTTP transport seam for OAuth provider adapters.
//!
//! [`HttpClient`] is an infrastructure-internal port (not a domain port — the
//! domain knows nothing of HTTP): [`OidcProvider`](crate::oauth_provider) does
//! its token/userinfo calls through it, so its request-building and
//! response-parsing are unit-testable against a fake transport, while
//! production wires the real [`ReqwestHttpClient`].

use async_trait::async_trait;

/// A minimal HTTP response: status code and raw body text.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response body, as text (JSON, for the endpoints we call).
    pub body: String,
}

impl HttpResponse {
    /// Whether the status is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A transport error (connection failure, TLS, unreadable body).
#[derive(Debug)]
pub struct HttpError(pub String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "http transport error: {}", self.0)
    }
}

impl std::error::Error for HttpError {}

/// Infrastructure-internal port: the two HTTP shapes an OIDC flow needs.
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// POST an `application/x-www-form-urlencoded` body (the token endpoint).
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
    ) -> Result<HttpResponse, HttpError>;

    /// GET with a `Bearer` token (the userinfo endpoint).
    async fn get_bearer(&self, url: &str, bearer: &str) -> Result<HttpResponse, HttpError>;
}

/// A [`HttpClient`] backed by `reqwest` (rustls). The production transport.
#[derive(Debug, Clone)]
pub struct ReqwestHttpClient {
    client: reqwest::Client,
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestHttpClient {
    /// Build the client with sensible timeouts.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("building a reqwest client with default TLS cannot fail");
        Self { client }
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
    ) -> Result<HttpResponse, HttpError> {
        let response = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|e| HttpError(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| HttpError(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }

    async fn get_bearer(&self, url: &str, bearer: &str) -> Result<HttpResponse, HttpError> {
        let response = self
            .client
            .get(url)
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(|e| HttpError(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| HttpError(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}
