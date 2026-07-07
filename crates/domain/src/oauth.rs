//! OAuth2 / OIDC authorization-code + PKCE sign-in: the model and the ports
//! for logging an administrator in through an external identity provider,
//! without coupling the domain to any specific vendor.
//!
//! Following the same hexagonal discipline as the rest of the domain: pure
//! value objects plus *ports* (traits) the outer layers implement — the
//! [`OAuthProvider`] a provider adapter fulfils (Google, GitHub, any OIDC
//! server), the [`PendingAuthStore`] and [`OAuthIdentityRepository`] a Postgres
//! adapter fulfils, and the [`OAuthSecretGenerator`] a CSPRNG adapter fulfils.
//! No HTTP client, no database driver, no JWT library reaches in here.

use std::time::SystemTime;

use crate::{AdminId, RepositoryError};

/// A provider's stable identifier (`"google"`, `"github"`, ...). Normalized
/// (trimmed, lowercased) so it is a stable lookup/display key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    /// Parse and normalize a provider id.
    pub fn parse(raw: &str) -> Result<Self, OAuthError> {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Err(OAuthError::Config("provider id must not be empty".into()));
        }
        Ok(Self(normalized))
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A PKCE pair: the high-entropy `verifier` kept server-side, and the
/// `challenge` (its `S256` transform) sent in the authorize request. The
/// provider ties the two together at the token endpoint, so an attacker who
/// intercepts the authorization code cannot redeem it without the verifier.
///
/// Held as opaque strings; the `S256` transform itself lives in the
/// [`OAuthSecretGenerator`] adapter (it needs SHA-256, which the domain does
/// not depend on).
#[derive(Clone)]
pub struct PkcePair {
    verifier: String,
    challenge: String,
}

impl PkcePair {
    /// Wrap a freshly generated verifier and its matching `S256` challenge.
    pub fn new(verifier: impl Into<String>, challenge: impl Into<String>) -> Self {
        Self {
            verifier: verifier.into(),
            challenge: challenge.into(),
        }
    }

    /// The verifier, to persist and later send to the token endpoint.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// The `S256` challenge, to put in the authorize URL.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

// The verifier is a bearer secret; keep it out of logs.
impl std::fmt::Debug for PkcePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PkcePair(<redacted>)")
    }
}

/// The parameters an [`OAuthProvider`] needs to build an authorize URL.
#[derive(Debug, Clone)]
pub struct AuthorizeParams {
    /// The anti-CSRF `state`, echoed back on the callback and matched against
    /// the stored [`PendingAuthorization`].
    pub state: String,
    /// The OIDC `nonce`, echoed inside the id_token and matched there, so a
    /// replayed id_token from a different flow is rejected.
    pub nonce: String,
    /// The PKCE `S256` challenge.
    pub code_challenge: String,
    /// Where the provider should send the browser back to.
    pub redirect_uri: String,
}

/// A request to exchange an authorization code for the caller's identity.
#[derive(Debug, Clone)]
pub struct ExchangeRequest {
    /// The `code` the provider returned on the callback.
    pub code: String,
    /// The PKCE verifier matching the challenge sent at authorize time.
    pub code_verifier: String,
    /// The redirect URI (must match the one used at authorize time).
    pub redirect_uri: String,
    /// The `nonce` the provider must echo in the id_token.
    pub expected_nonce: String,
}

/// The external identity an [`OAuthProvider`] resolves a code to — deliberately
/// just the provider's stable subject id and the account's email. The provider
/// adapter never hands the application layer an access token or id_token, which
/// is what keeps "tokens never exposed to the frontend" true by construction:
/// there is no token in this type for a delivery layer to leak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthIdentity {
    /// Which provider this identity came from.
    pub provider: ProviderId,
    /// The provider's stable, unique subject identifier for the user.
    pub subject: String,
    /// The verified email the provider reports for the user.
    pub email: String,
}

/// Port: one external identity provider (a Google/GitHub/OIDC adapter).
///
/// Adding a provider is implementing this trait (or configuring the generic
/// OIDC adapter) plus registering it — the application flow and the domain
/// never change. Implementations MUST NOT surface any token or vendor SDK
/// type through this trait; everything crossing the boundary is one of this
/// module's own types.
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    /// This provider's id, matching the `:provider` path segment.
    fn id(&self) -> ProviderId;

    /// Build the provider's authorize URL from `params`. Pure — no I/O.
    fn authorize_url(&self, params: &AuthorizeParams) -> String;

    /// Exchange an authorization code for the caller's external identity:
    /// redeem the code at the token endpoint (sending the PKCE verifier),
    /// validate the id_token's `nonce`, and resolve the subject and email.
    async fn exchange_code(&self, request: &ExchangeRequest) -> Result<OAuthIdentity, OAuthError>;
}

/// A pending authorization: everything a callback needs to finish a flow that
/// a `begin` started, keyed by its one-time `state`.
#[derive(Debug, Clone)]
pub struct PendingAuthorization {
    /// The anti-CSRF state (the row's key).
    pub state: String,
    /// Which provider the flow is against.
    pub provider: ProviderId,
    /// The nonce to require in the id_token.
    pub nonce: String,
    /// The PKCE verifier to send at the token endpoint.
    pub code_verifier: String,
    /// The redirect URI used at authorize time.
    pub redirect_uri: String,
    /// When the flow started, for expiry.
    pub created_at: SystemTime,
}

/// Port: persistence for pending authorizations.
///
/// [`Self::consume`] is a one-shot take: it returns the row *and* deletes it in
/// one step, so a `state` cannot be replayed — the second callback with the
/// same state finds nothing.
#[async_trait::async_trait]
pub trait PendingAuthStore: Send + Sync {
    /// Persist a freshly started authorization.
    async fn insert(&self, pending: &PendingAuthorization) -> Result<(), RepositoryError>;

    /// Atomically fetch-and-delete the pending authorization for `state`.
    /// Returns `None` if there is none (unknown or already-consumed state).
    async fn consume(&self, state: &str) -> Result<Option<PendingAuthorization>, RepositoryError>;
}

/// Port: persistence for the link between an external identity and a local
/// admin account (the "own table" the acceptance criteria call for).
#[async_trait::async_trait]
pub trait OAuthIdentityRepository: Send + Sync {
    /// The admin an external `(provider, subject)` is linked to, if any.
    async fn find_admin(
        &self,
        provider: &ProviderId,
        subject: &str,
    ) -> Result<Option<AdminId>, RepositoryError>;

    /// Link an external `(provider, subject)` to a local admin. Idempotent:
    /// re-linking the same pair to the same admin is not an error.
    async fn link(
        &self,
        provider: &ProviderId,
        subject: &str,
        email: &str,
        admin_id: &AdminId,
    ) -> Result<(), RepositoryError>;
}

/// Port: generates the random secrets an OAuth flow needs (state, nonce, and
/// the PKCE pair). A port for the same reason [`crate::PasswordHasher`] is one:
/// the domain and application layers stay free of any concrete crypto crate.
pub trait OAuthSecretGenerator: Send + Sync {
    /// A fresh, unguessable anti-CSRF `state`.
    fn state(&self) -> String;

    /// A fresh, unguessable OIDC `nonce`.
    fn nonce(&self) -> String;

    /// A fresh PKCE verifier and its `S256` challenge.
    fn pkce(&self) -> PkcePair;
}

/// Why an OAuth operation failed.
#[derive(Debug)]
pub enum OAuthError {
    /// A configuration problem (unknown provider, malformed endpoint, ...).
    Config(String),
    /// The callback's `state` did not match any pending authorization —
    /// unknown, expired, or already consumed (a replay).
    InvalidState,
    /// The provider rejected the code exchange, or the id_token's `nonce`,
    /// `iss`, `aud`, or `exp` did not validate.
    ExchangeRejected(String),
    /// The resolved external identity is not linked to any local admin, and no
    /// local admin has its email — sign-in is refused rather than silently
    /// provisioning a new administrator.
    NoLinkedAccount,
    /// The provider could not be reached, or returned an unusable response.
    Provider(String),
    /// A storage failure.
    Internal(String),
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(m) => write!(f, "oauth configuration error: {m}"),
            Self::InvalidState => f.write_str("oauth state is invalid, expired, or already used"),
            Self::ExchangeRejected(m) => write!(f, "oauth code exchange rejected: {m}"),
            Self::NoLinkedAccount => f.write_str("no local admin is linked to this identity"),
            Self::Provider(m) => write!(f, "oauth provider error: {m}"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for OAuthError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_normalizes() {
        assert_eq!(ProviderId::parse("  Google ").unwrap().as_str(), "google");
        assert!(ProviderId::parse("").is_err());
    }

    #[test]
    fn pkce_pair_exposes_its_parts_but_redacts_debug() {
        let pair = PkcePair::new("the-verifier", "the-challenge");
        assert_eq!(pair.verifier(), "the-verifier");
        assert_eq!(pair.challenge(), "the-challenge");
        assert_eq!(format!("{pair:?}"), "PkcePair(<redacted>)");
    }
}
