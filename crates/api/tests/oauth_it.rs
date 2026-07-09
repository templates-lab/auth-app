//! End-to-end test for OAuth/OIDC sign-in (bead authapp-541886): drives the
//! real router through the real, config-driven [`OidcProvider`] — its token
//! form, its id_token nonce/issuer/audience/expiry validation, its userinfo
//! parsing — against a real, ephemeral, freshly migrated Postgres (pending
//! store, identity linking, session issuance). Only the socket to the provider
//! is faked, via a scripted [`HttpClient`]; everything the provider adapter
//! actually does runs for real.
//!
//! Secrets are fixed (a test-only [`OAuthSecretGenerator`]) so the fake token
//! response can carry an id_token whose `nonce` matches the one `begin`
//! generated — the whole point of the nonce check.

use std::sync::Arc;

use application::{AuditService, HealthService, LoginService, OAuthLoginService, SessionService};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use domain::{
    AdminRepository, Email, NewAdmin, OAuthProvider, OAuthSecretGenerator, PasswordHash, PkcePair,
    ProviderId, Role,
};
use infrastructure::{
    oauth_http::{HttpClient, HttpError, HttpResponse},
    Argon2Hasher, Argon2Params, OidcConfig, OidcProvider, PgAdminRepository, PgAuditRepository,
    PgHealthCheck, PgIpLockoutStore, PgOAuthIdentityRepository, PgPendingAuthStore,
    PgSessionRepository, SecureRandomTokens, SystemClock,
};
use tower::ServiceExt;

const ADMIN_EMAIL: &str = "admin@example.com";
const FIXED_STATE: &str = "fixed-state-value";
const FIXED_NONCE: &str = "fixed-nonce-value";
const CLIENT_ID: &str = "client-123";
const ISSUER: &str = "https://accounts.example.com";

/// Fixed secrets so the scripted id_token's nonce can match.
struct FixedSecrets;
impl OAuthSecretGenerator for FixedSecrets {
    fn state(&self) -> String {
        FIXED_STATE.to_string()
    }
    fn nonce(&self) -> String {
        FIXED_NONCE.to_string()
    }
    fn pkce(&self) -> PkcePair {
        PkcePair::new("fixed-verifier", "fixed-challenge")
    }
}

/// A scripted transport: the token endpoint returns an id_token carrying
/// `FIXED_NONCE`, the userinfo endpoint returns the admin's identity.
struct ScriptedHttp;

fn fake_id_token() -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#);
    let payload = serde_json::json!({
        "nonce": FIXED_NONCE,
        "iss": ISSUER,
        "aud": CLIENT_ID,
        "exp": 9_999_999_999i64,
    });
    let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
    format!("{header}.{payload}.signature")
}

#[async_trait::async_trait]
impl HttpClient for ScriptedHttp {
    async fn post_form(
        &self,
        _url: &str,
        _form: &[(String, String)],
        _bearer: Option<&str>,
    ) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: 200,
            body: serde_json::json!({
                "access_token": "fake-access-token",
                "id_token": fake_id_token(),
            })
            .to_string(),
        })
    }
    async fn get_bearer(&self, _url: &str, _bearer: &str) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: 200,
            body: serde_json::json!({
                "sub": "google-subject-1",
                "email": ADMIN_EMAIL,
            })
            .to_string(),
        })
    }
}

fn oidc_config() -> OidcConfig {
    OidcConfig {
        provider_id: ProviderId::parse("google").unwrap(),
        client_id: CLIENT_ID.to_string(),
        client_secret: "secret".to_string(),
        auth_endpoint: "https://accounts.example.com/authorize".to_string(),
        token_endpoint: "https://accounts.example.com/token".to_string(),
        userinfo_endpoint: "https://accounts.example.com/userinfo".to_string(),
        issuer: ISSUER.to_string(),
        scopes: vec!["openid".to_string(), "email".to_string()],
    }
}

async fn router(pool: sqlx::PgPool) -> axum::Router {
    let health = HealthService::new(Arc::new(PgHealthCheck::new(pool.clone())));
    let login = LoginService::new(
        Arc::new(PgAdminRepository::new(pool.clone())),
        Arc::new(PgIpLockoutStore::new(pool.clone())),
        Arc::new(Argon2Hasher::new(Argon2Params::owasp_default()).unwrap()),
        Arc::new(SystemClock),
        domain::LockoutPolicy::recommended(),
    );
    let sessions = SessionService::new(
        Arc::new(PgSessionRepository::new(pool.clone())),
        Arc::new(SecureRandomTokens),
        Arc::new(SystemClock),
        domain::SessionPolicy::recommended(),
    );
    let audit = AuditService::new(Arc::new(PgAuditRepository::new(pool.clone())));

    let provider: Arc<dyn OAuthProvider> =
        Arc::new(OidcProvider::new(oidc_config(), Arc::new(ScriptedHttp)));
    let oauth = OAuthLoginService::new(
        vec![provider],
        Arc::new(PgPendingAuthStore::new(pool.clone())),
        Arc::new(PgOAuthIdentityRepository::new(pool.clone())),
        Arc::new(PgAdminRepository::new(pool.clone())),
        Arc::new(FixedSecrets),
        Arc::new(SystemClock),
        "https://app.example.com",
    );

    api::router(
        health,
        login,
        sessions,
        audit,
        Arc::new(PgAdminRepository::new(pool.clone())),
        Some((oauth, api::oauth::OAuthRedirects::default())),
        None,
        None,
        &[],
        api::rate_limit::RateLimitConfig {
            max_requests: 100,
            window: std::time::Duration::from_secs(60),
        },
    )
}

async fn seed_admin(pool: &sqlx::PgPool) {
    // The account exists so OAuth can link/authenticate against it; its
    // password is never used on the OAuth path.
    let password_hash = PasswordHash::from_encoded("argon2-placeholder");
    PgAdminRepository::new(pool.clone())
        .insert(&NewAdmin {
            email: Email::parse(ADMIN_EMAIL).unwrap(),
            password_hash,
            role: Role::admin(),
        })
        .await
        .unwrap();
}

fn location(response: &axum::http::Response<Body>) -> String {
    response
        .headers()
        .get("location")
        .expect("a redirect must carry a Location header")
        .to_str()
        .unwrap()
        .to_string()
}

fn has_session_cookie(response: &axum::http::Response<Body>) -> bool {
    response.headers().get_all("set-cookie").iter().any(|v| {
        v.to_str()
            .map(|s| s.starts_with("session="))
            .unwrap_or(false)
    })
}

#[tokio::test]
async fn start_redirects_to_the_provider_with_pkce_and_state() {
    let db = testkit::spawn_test_db().await;
    let app = router(db.pool.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/google/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_redirection());
    let url = location(&response);
    assert!(url.starts_with("https://accounts.example.com/authorize?"));
    assert!(url.contains(&format!("state={FIXED_STATE}")));
    assert!(url.contains("code_challenge=fixed-challenge"));
    assert!(url.contains("code_challenge_method=S256"));
    // The redirect_uri is derived from the configured base + provider.
    assert!(url
        .contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fauth%2Foauth%2Fgoogle%2Fcallback"));
}

#[tokio::test]
async fn providers_lists_the_configured_provider_ids() {
    let db = testkit::spawn_test_db().await;
    let app = router(db.pool.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["providers"], serde_json::json!(["google"]));
}

#[tokio::test]
async fn start_for_an_unknown_provider_is_404() {
    let db = testkit::spawn_test_db().await;
    let app = router(db.pool.clone()).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/facebook/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn full_flow_links_the_identity_and_issues_a_session() {
    let db = testkit::spawn_test_db().await;
    seed_admin(&db.pool).await;
    let app = router(db.pool.clone()).await;

    // 1. Start: persists the pending authorization (state -> nonce/verifier).
    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/google/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(start.status().is_redirection());

    // 2. Callback: the real OidcProvider exchanges the code (validating the
    //    id_token nonce/iss/aud/exp) and userinfo, links the identity, and a
    //    session is issued.
    let callback = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/auth/oauth/google/callback?state={FIXED_STATE}&code=the-code"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(callback.status().is_redirection());
    assert_eq!(location(&callback), "/");
    assert!(
        has_session_cookie(&callback),
        "a successful callback must set the session cookie"
    );

    // The external identity is now linked in its own table.
    let linked: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM admin_oauth_identities WHERE subject = $1")
            .bind("google-subject-1")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(linked, 1);
}

#[tokio::test]
async fn callback_with_an_unknown_state_redirects_to_failure_without_a_session() {
    let db = testkit::spawn_test_db().await;
    seed_admin(&db.pool).await;
    let app = router(db.pool.clone()).await;

    // No `start` first, so this state was never issued.
    let callback = app
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/google/callback?state=never-issued&code=the-code")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(callback.status().is_redirection());
    assert!(location(&callback).starts_with("/login"));
    assert!(location(&callback).contains("error=oauth"));
    assert!(!has_session_cookie(&callback));
}

#[tokio::test]
async fn callback_with_a_provider_error_redirects_to_failure() {
    let db = testkit::spawn_test_db().await;
    let app = router(db.pool.clone()).await;

    let callback = app
        .oneshot(
            Request::builder()
                .uri("/auth/oauth/google/callback?error=access_denied")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(callback.status().is_redirection());
    assert!(location(&callback).contains("error=oauth"));
}
