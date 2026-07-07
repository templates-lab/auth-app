//! A generic, config-driven OIDC provider adapter implementing the
//! [`OAuthProvider`] port.
//!
//! One adapter serves any standard OIDC provider (Google, an in-house
//! Keycloak, ...): pointing it at a new provider is *configuration*
//! ([`OidcConfig`] — endpoints, client id/secret, scopes), never new code,
//! which is exactly the "add a provider = config, without touching the core"
//! acceptance criterion. A provider that is not OIDC (GitHub, say) is a new
//! `impl OAuthProvider` next to this one — the trait, not this struct, is the
//! extension point.
//!
//! The token/userinfo HTTP calls go through the [`HttpClient`] seam, so this
//! adapter's request-building and response-parsing are unit-tested against a
//! fake transport (no network); production injects [`ReqwestHttpClient`].

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use domain::{
    AuthorizeParams, ExchangeRequest, OAuthError, OAuthIdentity, OAuthProvider, ProviderId,
};
use serde::Deserialize;

use crate::oauth_http::HttpClient;

/// Configuration for one OIDC provider. Everything here is per-provider and
/// comes from the environment — swapping providers never recompiles.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// The provider id (`"google"`), matching the callback path segment.
    pub provider_id: ProviderId,
    /// The OAuth client id.
    pub client_id: String,
    /// The OAuth client secret.
    pub client_secret: String,
    /// The authorization endpoint (where the browser is sent).
    pub auth_endpoint: String,
    /// The token endpoint (server-to-server code exchange).
    pub token_endpoint: String,
    /// The userinfo endpoint (server-to-server, bearer access token).
    pub userinfo_endpoint: String,
    /// The `iss` the id_token must carry (issuer validation).
    pub issuer: String,
    /// The scopes to request (`openid`, `email`, ...).
    pub scopes: Vec<String>,
}

/// A generic OIDC [`OAuthProvider`].
pub struct OidcProvider {
    config: OidcConfig,
    http: Arc<dyn HttpClient>,
}

impl std::fmt::Debug for OidcProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProvider")
            .field("provider_id", &self.config.provider_id)
            .field("issuer", &self.config.issuer)
            .finish_non_exhaustive()
    }
}

impl OidcProvider {
    /// Build the provider from its config and an HTTP transport.
    pub fn new(config: OidcConfig, http: Arc<dyn HttpClient>) -> Self {
        Self { config, http }
    }
}

/// The subset of the token response we consume.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    id_token: String,
}

/// The id_token claims we validate.
#[derive(Debug, Deserialize)]
struct IdTokenClaims {
    iss: String,
    #[serde(default)]
    aud: Audience,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
}

/// `aud` may be a single string or an array of strings.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
    #[default]
    None,
}

impl Audience {
    fn contains(&self, value: &str) -> bool {
        match self {
            Self::One(s) => s == value,
            Self::Many(v) => v.iter().any(|s| s == value),
            Self::None => false,
        }
    }
}

/// The subset of the userinfo response we consume.
#[derive(Debug, Deserialize)]
struct UserInfo {
    sub: String,
    email: String,
}

#[async_trait]
impl OAuthProvider for OidcProvider {
    fn id(&self) -> ProviderId {
        self.config.provider_id.clone()
    }

    fn authorize_url(&self, params: &AuthorizeParams) -> String {
        // Build with `reqwest::Url` so every value is correctly percent-encoded.
        let scope = self.config.scopes.join(" ");
        let query = [
            ("response_type", "code"),
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", params.redirect_uri.as_str()),
            ("scope", scope.as_str()),
            ("state", params.state.as_str()),
            ("nonce", params.nonce.as_str()),
            ("code_challenge", params.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        match reqwest::Url::parse_with_params(&self.config.auth_endpoint, query) {
            Ok(url) => url.to_string(),
            // A malformed configured endpoint is a config error; surfacing it
            // as an obviously-broken URL is fine here since `begin` validated
            // the provider exists, and the browser redirect will simply fail
            // loudly rather than silently.
            Err(_) => self.config.auth_endpoint.clone(),
        }
    }

    async fn exchange_code(&self, request: &ExchangeRequest) -> Result<OAuthIdentity, OAuthError> {
        // 1. Redeem the code (with the PKCE verifier) at the token endpoint.
        let form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), request.code.clone()),
            ("redirect_uri".to_string(), request.redirect_uri.clone()),
            ("client_id".to_string(), self.config.client_id.clone()),
            (
                "client_secret".to_string(),
                self.config.client_secret.clone(),
            ),
            ("code_verifier".to_string(), request.code_verifier.clone()),
        ];
        let response = self
            .http
            .post_form(&self.config.token_endpoint, &form)
            .await
            .map_err(|e| OAuthError::Provider(e.to_string()))?;
        if !response.is_success() {
            return Err(OAuthError::ExchangeRejected(format!(
                "token endpoint returned {}",
                response.status
            )));
        }
        let token: TokenResponse = serde_json::from_str(&response.body)
            .map_err(|e| OAuthError::Provider(format!("token response: {e}")))?;

        // 2. Validate the id_token's claims (nonce, issuer, audience, expiry).
        //    NOTE: this decodes the id_token payload and validates its claims
        //    but does not yet verify its RS256 signature against the provider's
        //    JWKS. That is safe *here* because the id_token arrives directly
        //    from the token endpoint over TLS (not via the browser), and the
        //    identity of record below comes from the userinfo endpoint (also a
        //    direct, bearer-authenticated TLS call). JWKS signature
        //    verification is the documented remaining hardening step.
        self.validate_id_token(&token.id_token, &request.expected_nonce)?;

        // 3. The identity of record comes from userinfo (subject + email).
        let userinfo = self
            .http
            .get_bearer(&self.config.userinfo_endpoint, &token.access_token)
            .await
            .map_err(|e| OAuthError::Provider(e.to_string()))?;
        if !userinfo.is_success() {
            return Err(OAuthError::Provider(format!(
                "userinfo endpoint returned {}",
                userinfo.status
            )));
        }
        let info: UserInfo = serde_json::from_str(&userinfo.body)
            .map_err(|e| OAuthError::Provider(format!("userinfo response: {e}")))?;

        Ok(OAuthIdentity {
            provider: self.config.provider_id.clone(),
            subject: info.sub,
            email: info.email,
        })
    }
}

impl OidcProvider {
    /// Decode the id_token payload and validate `nonce`, `iss`, `aud`, `exp`.
    fn validate_id_token(&self, id_token: &str, expected_nonce: &str) -> Result<(), OAuthError> {
        let payload = id_token
            .split('.')
            .nth(1)
            .ok_or_else(|| OAuthError::ExchangeRejected("id_token is not a JWT".into()))?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| OAuthError::ExchangeRejected(format!("id_token payload: {e}")))?;
        let claims: IdTokenClaims = serde_json::from_slice(&bytes)
            .map_err(|e| OAuthError::ExchangeRejected(format!("id_token claims: {e}")))?;

        match claims.nonce.as_deref() {
            Some(n) if n == expected_nonce => {}
            _ => {
                return Err(OAuthError::ExchangeRejected(
                    "id_token nonce mismatch".into(),
                ))
            }
        }
        if claims.iss != self.config.issuer {
            return Err(OAuthError::ExchangeRejected(
                "id_token issuer mismatch".into(),
            ));
        }
        if !claims.aud.contains(&self.config.client_id) {
            return Err(OAuthError::ExchangeRejected(
                "id_token audience mismatch".into(),
            ));
        }
        if let Some(exp) = claims.exp {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if exp <= now {
                return Err(OAuthError::ExchangeRejected("id_token has expired".into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::oauth_http::{HttpError, HttpResponse};

    fn config() -> OidcConfig {
        OidcConfig {
            provider_id: ProviderId::parse("google").unwrap(),
            client_id: "client-123".to_string(),
            client_secret: "secret-xyz".to_string(),
            auth_endpoint: "https://accounts.example.com/authorize".to_string(),
            token_endpoint: "https://accounts.example.com/token".to_string(),
            userinfo_endpoint: "https://accounts.example.com/userinfo".to_string(),
            issuer: "https://accounts.example.com".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string()],
        }
    }

    /// Build a fake id_token: a JWT with an unsigned header and the given
    /// payload claims (only the payload is read).
    fn fake_id_token(nonce: &str, iss: &str, aud: &str, exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = serde_json::json!({
            "nonce": nonce, "iss": iss, "aud": aud, "exp": exp
        });
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{payload}.sig")
    }

    /// A scripted HTTP client: returns the queued token response for the POST
    /// and the queued userinfo response for the GET.
    struct ScriptedHttp {
        token: HttpResponse,
        userinfo: HttpResponse,
        posted_form: Mutex<Option<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl HttpClient for ScriptedHttp {
        async fn post_form(
            &self,
            _url: &str,
            form: &[(String, String)],
        ) -> Result<HttpResponse, HttpError> {
            *self.posted_form.lock().unwrap() = Some(form.to_vec());
            Ok(self.token.clone())
        }
        async fn get_bearer(&self, _url: &str, _bearer: &str) -> Result<HttpResponse, HttpError> {
            Ok(self.userinfo.clone())
        }
    }

    fn provider(token_body: String, userinfo_body: String) -> (OidcProvider, Arc<ScriptedHttp>) {
        let http = Arc::new(ScriptedHttp {
            token: HttpResponse {
                status: 200,
                body: token_body,
            },
            userinfo: HttpResponse {
                status: 200,
                body: userinfo_body,
            },
            posted_form: Mutex::new(None),
        });
        (OidcProvider::new(config(), http.clone()), http)
    }

    #[test]
    fn authorize_url_carries_every_pkce_and_oidc_param() {
        let (provider, _) = provider(String::new(), String::new());
        let url = provider.authorize_url(&AuthorizeParams {
            state: "st".to_string(),
            nonce: "no".to_string(),
            code_challenge: "ch".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
        });
        assert!(url.starts_with("https://accounts.example.com/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("code_challenge=ch"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st"));
        assert!(url.contains("nonce=no"));
        // The redirect URI is percent-encoded.
        assert!(url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fcb"));
    }

    fn exchange_request(nonce: &str) -> ExchangeRequest {
        ExchangeRequest {
            code: "the-code".to_string(),
            code_verifier: "the-verifier".to_string(),
            redirect_uri: "https://app.example.com/cb".to_string(),
            expected_nonce: nonce.to_string(),
        }
    }

    #[tokio::test]
    async fn exchange_returns_the_userinfo_identity_and_sends_pkce_verifier() {
        let id_token = fake_id_token(
            "expected-nonce",
            "https://accounts.example.com",
            "client-123",
            9_999_999_999,
        );
        let token_body =
            serde_json::json!({"access_token": "at", "id_token": id_token}).to_string();
        let userinfo_body =
            serde_json::json!({"sub": "google-sub-1", "email": "admin@example.com"}).to_string();
        let (provider, http) = provider(token_body, userinfo_body);

        let identity = provider
            .exchange_code(&exchange_request("expected-nonce"))
            .await
            .unwrap();
        assert_eq!(identity.subject, "google-sub-1");
        assert_eq!(identity.email, "admin@example.com");
        assert_eq!(identity.provider, ProviderId::parse("google").unwrap());

        // The token POST carried the PKCE verifier (the whole point of PKCE).
        let form = http.posted_form.lock().unwrap().clone().unwrap();
        assert!(form
            .iter()
            .any(|(k, v)| k == "code_verifier" && v == "the-verifier"));
        assert!(form
            .iter()
            .any(|(k, v)| k == "grant_type" && v == "authorization_code"));
    }

    #[tokio::test]
    async fn exchange_rejects_a_nonce_mismatch() {
        let id_token = fake_id_token(
            "a-different-nonce",
            "https://accounts.example.com",
            "client-123",
            9_999_999_999,
        );
        let token_body =
            serde_json::json!({"access_token": "at", "id_token": id_token}).to_string();
        let (provider, _) = provider(token_body, String::new());

        assert!(matches!(
            provider
                .exchange_code(&exchange_request("expected-nonce"))
                .await,
            Err(OAuthError::ExchangeRejected(_))
        ));
    }

    #[tokio::test]
    async fn exchange_rejects_a_wrong_audience() {
        let id_token = fake_id_token(
            "expected-nonce",
            "https://accounts.example.com",
            "some-other-client",
            9_999_999_999,
        );
        let token_body =
            serde_json::json!({"access_token": "at", "id_token": id_token}).to_string();
        let (provider, _) = provider(token_body, String::new());

        assert!(matches!(
            provider
                .exchange_code(&exchange_request("expected-nonce"))
                .await,
            Err(OAuthError::ExchangeRejected(_))
        ));
    }

    #[tokio::test]
    async fn exchange_rejects_an_expired_id_token() {
        let id_token = fake_id_token(
            "expected-nonce",
            "https://accounts.example.com",
            "client-123",
            1, // 1970 — long expired
        );
        let token_body =
            serde_json::json!({"access_token": "at", "id_token": id_token}).to_string();
        let (provider, _) = provider(token_body, String::new());

        assert!(matches!(
            provider
                .exchange_code(&exchange_request("expected-nonce"))
                .await,
            Err(OAuthError::ExchangeRejected(_))
        ));
    }
}
