//! OAuth sign-in use cases: begin a flow (build the authorize URL and stash a
//! pending authorization) and complete it (validate `state`, exchange the
//! code, resolve the external identity to a local admin, and hand the delivery
//! layer the [`AdminId`]/[`Role`] to issue a session for).
//!
//! [`OAuthLoginService`] holds no knowledge of HTTP or storage; it wires the
//! domain ports the composition root injects. Crucially it never returns —
//! and its ports never surface — an access token or id_token, so the delivery
//! layer has nothing token-shaped to leak to the frontend.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use domain::{
    AdminId, AdminRepository, AuthorizeParams, Clock, Email, ExchangeRequest, OAuthError,
    OAuthIdentity, OAuthIdentityRepository, OAuthProvider, OAuthSecretGenerator, PendingAuthStore,
    PendingAuthorization, ProviderId, Role,
};

/// What [`OAuthLoginService::begin`] hands back: the URL to redirect the
/// browser to. The `state`/`nonce`/verifier are already persisted server-side,
/// so nothing secret needs to travel back through the delivery layer.
#[derive(Debug, Clone)]
pub struct BeginOutcome {
    /// The provider authorize URL to redirect the browser to.
    pub authorize_url: String,
}

/// The identity a completed OAuth flow resolves to — the same shape
/// [`crate::LoginService`] returns, so the delivery layer issues a session the
/// same way for a password login and an OAuth login.
#[derive(Debug, Clone)]
pub struct OAuthAuthenticatedAdmin {
    /// The local admin the external identity maps to.
    pub id: AdminId,
    /// That admin's role.
    pub role: Role,
}

/// How long a started flow may sit before its pending authorization expires.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// Application service for OAuth sign-in.
#[derive(Clone)]
pub struct OAuthLoginService {
    providers: Arc<HashMap<String, Arc<dyn OAuthProvider>>>,
    pending: Arc<dyn PendingAuthStore>,
    identities: Arc<dyn OAuthIdentityRepository>,
    admins: Arc<dyn AdminRepository>,
    secrets: Arc<dyn OAuthSecretGenerator>,
    clock: Arc<dyn Clock>,
    redirect_base: String,
}

impl std::fmt::Debug for OAuthLoginService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthLoginService")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl OAuthLoginService {
    /// Assemble the service from its ports and the registered providers.
    ///
    /// `redirect_base` is the externally reachable origin (e.g.
    /// `https://admin.example.com`); the per-provider callback URL is
    /// `{redirect_base}/auth/oauth/{provider}/callback`.
    pub fn new(
        providers: Vec<Arc<dyn OAuthProvider>>,
        pending: Arc<dyn PendingAuthStore>,
        identities: Arc<dyn OAuthIdentityRepository>,
        admins: Arc<dyn AdminRepository>,
        secrets: Arc<dyn OAuthSecretGenerator>,
        clock: Arc<dyn Clock>,
        redirect_base: impl Into<String>,
    ) -> Self {
        let providers = providers
            .into_iter()
            .map(|p| (p.id().as_str().to_string(), p))
            .collect();
        Self {
            providers: Arc::new(providers),
            pending,
            identities,
            admins,
            secrets,
            clock,
            redirect_base: redirect_base.into(),
        }
    }

    /// Whether a provider id is configured — lets the delivery layer answer a
    /// `404` for an unknown provider before doing anything else.
    pub fn has_provider(&self, provider: &str) -> bool {
        self.providers.contains_key(provider)
    }

    fn provider(&self, provider: &ProviderId) -> Result<&Arc<dyn OAuthProvider>, OAuthError> {
        self.providers
            .get(provider.as_str())
            .ok_or_else(|| OAuthError::Config(format!("unknown provider {provider}")))
    }

    fn redirect_uri(&self, provider: &ProviderId) -> String {
        format!(
            "{}/auth/oauth/{}/callback",
            self.redirect_base.trim_end_matches('/'),
            provider
        )
    }

    /// Begin a flow: generate `state`/`nonce`/PKCE, persist the pending
    /// authorization, and return the provider authorize URL.
    pub async fn begin(&self, provider_raw: &str) -> Result<BeginOutcome, OAuthError> {
        let provider_id = ProviderId::parse(provider_raw)?;
        let provider = self.provider(&provider_id)?;

        let state = self.secrets.state();
        let nonce = self.secrets.nonce();
        let pkce = self.secrets.pkce();
        let redirect_uri = self.redirect_uri(&provider_id);

        self.pending
            .insert(&PendingAuthorization {
                state: state.clone(),
                provider: provider_id.clone(),
                nonce: nonce.clone(),
                code_verifier: pkce.verifier().to_string(),
                redirect_uri: redirect_uri.clone(),
                created_at: self.clock.now(),
            })
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?;

        let authorize_url = provider.authorize_url(&AuthorizeParams {
            state,
            nonce,
            code_challenge: pkce.challenge().to_string(),
            redirect_uri,
        });
        Ok(BeginOutcome { authorize_url })
    }

    /// Complete a flow from the callback's `state` and `code`.
    ///
    /// Validates `state` (one-shot consume — a replay finds nothing), exchanges
    /// the code (which validates the id_token `nonce`), resolves the external
    /// identity to a local admin, and returns that admin for the delivery layer
    /// to issue a session for. A first-time identity whose email matches an
    /// existing admin is linked then; an identity with no link and no
    /// matching admin is refused ([`OAuthError::NoLinkedAccount`]) — OAuth
    /// never silently provisions a new administrator.
    pub async fn complete(
        &self,
        state: &str,
        code: &str,
    ) -> Result<OAuthAuthenticatedAdmin, OAuthError> {
        let pending = self
            .pending
            .consume(state)
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?
            .ok_or(OAuthError::InvalidState)?;

        // A pending authorization older than the TTL is treated as invalid,
        // same as an unknown state — it was already removed by `consume`.
        if self
            .clock
            .now()
            .duration_since(pending.created_at)
            .map(|age| age > PENDING_TTL)
            .unwrap_or(true)
        {
            return Err(OAuthError::InvalidState);
        }

        let provider = self.provider(&pending.provider)?;
        let identity = provider
            .exchange_code(&ExchangeRequest {
                code: code.to_string(),
                code_verifier: pending.code_verifier,
                redirect_uri: pending.redirect_uri,
                expected_nonce: pending.nonce,
            })
            .await?;

        let admin_id = self.resolve_admin(&identity).await?;

        // Fetch the account for its role (and to confirm it still exists).
        let account = self
            .admins
            .find_by_email(
                &Email::parse(&identity.email)
                    .map_err(|e| OAuthError::Provider(format!("provider email: {e}")))?,
            )
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?
            .ok_or(OAuthError::NoLinkedAccount)?;

        Ok(OAuthAuthenticatedAdmin {
            id: admin_id,
            role: account.role,
        })
    }

    /// Map an external identity to a local admin: an existing link wins; else
    /// an admin with the same email is linked now; else refuse.
    async fn resolve_admin(&self, identity: &OAuthIdentity) -> Result<AdminId, OAuthError> {
        if let Some(admin_id) = self
            .identities
            .find_admin(&identity.provider, &identity.subject)
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?
        {
            return Ok(admin_id);
        }

        let email = Email::parse(&identity.email)
            .map_err(|e| OAuthError::Provider(format!("provider email: {e}")))?;
        let account = self
            .admins
            .find_by_email(&email)
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?
            .ok_or(OAuthError::NoLinkedAccount)?;

        self.identities
            .link(
                &identity.provider,
                &identity.subject,
                &identity.email,
                &account.id,
            )
            .await
            .map_err(|e| OAuthError::Internal(e.to_string()))?;
        Ok(account.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::SystemTime;

    use domain::{
        AdminAccount, AuthorizeParams, LockoutState, NewAdmin, PasswordHash, PkcePair,
        RepositoryError,
    };

    // A provider whose authorize URL is trivial and whose exchange returns a
    // fixed identity — enough to drive the flow without real HTTP.
    struct FakeProvider {
        id: ProviderId,
        identity: OAuthIdentity,
        last_request: Mutex<Option<ExchangeRequest>>,
    }

    #[async_trait::async_trait]
    impl OAuthProvider for FakeProvider {
        fn id(&self) -> ProviderId {
            self.id.clone()
        }
        fn authorize_url(&self, params: &AuthorizeParams) -> String {
            format!(
                "https://provider.test/authorize?state={}&challenge={}&nonce={}",
                params.state, params.code_challenge, params.nonce
            )
        }
        async fn exchange_code(
            &self,
            request: &ExchangeRequest,
        ) -> Result<OAuthIdentity, OAuthError> {
            *self.last_request.lock().unwrap() = Some(request.clone());
            Ok(self.identity.clone())
        }
    }

    #[derive(Default)]
    struct InMemoryPending {
        rows: Mutex<HashMap<String, PendingAuthorization>>,
    }
    #[async_trait::async_trait]
    impl PendingAuthStore for InMemoryPending {
        async fn insert(&self, pending: &PendingAuthorization) -> Result<(), RepositoryError> {
            self.rows
                .lock()
                .unwrap()
                .insert(pending.state.clone(), pending.clone());
            Ok(())
        }
        async fn consume(
            &self,
            state: &str,
        ) -> Result<Option<PendingAuthorization>, RepositoryError> {
            Ok(self.rows.lock().unwrap().remove(state))
        }
    }

    #[derive(Default)]
    struct InMemoryIdentities {
        links: Mutex<HashMap<(String, String), AdminId>>,
    }
    #[async_trait::async_trait]
    impl OAuthIdentityRepository for InMemoryIdentities {
        async fn find_admin(
            &self,
            provider: &ProviderId,
            subject: &str,
        ) -> Result<Option<AdminId>, RepositoryError> {
            Ok(self
                .links
                .lock()
                .unwrap()
                .get(&(provider.as_str().to_string(), subject.to_string()))
                .cloned())
        }
        async fn link(
            &self,
            provider: &ProviderId,
            subject: &str,
            _email: &str,
            admin_id: &AdminId,
        ) -> Result<(), RepositoryError> {
            self.links.lock().unwrap().insert(
                (provider.as_str().to_string(), subject.to_string()),
                admin_id.clone(),
            );
            Ok(())
        }
    }

    #[derive(Default)]
    struct InMemoryAdmins {
        accounts: Mutex<Vec<AdminAccount>>,
    }
    #[async_trait::async_trait]
    impl AdminRepository for InMemoryAdmins {
        async fn find_by_email(
            &self,
            email: &Email,
        ) -> Result<Option<AdminAccount>, RepositoryError> {
            Ok(self
                .accounts
                .lock()
                .unwrap()
                .iter()
                .find(|a| &a.email == email)
                .cloned())
        }
        async fn update_lockout(
            &self,
            _id: &AdminId,
            _state: &LockoutState,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }
        async fn count(&self) -> Result<u64, RepositoryError> {
            Ok(self.accounts.lock().unwrap().len() as u64)
        }
        async fn insert(&self, _admin: &NewAdmin) -> Result<AdminId, RepositoryError> {
            unimplemented!("not exercised by the oauth flow")
        }
    }

    struct FixedSecrets;
    impl OAuthSecretGenerator for FixedSecrets {
        fn state(&self) -> String {
            "state-123".to_string()
        }
        fn nonce(&self) -> String {
            "nonce-abc".to_string()
        }
        fn pkce(&self) -> PkcePair {
            PkcePair::new("verifier-xyz", "challenge-xyz")
        }
    }

    struct FixedClock(SystemTime);
    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            self.0
        }
    }

    fn seed_admin(admins: &InMemoryAdmins, email: &str, role: Role) -> AdminId {
        let id = AdminId::new(format!("id-{email}"));
        admins.accounts.lock().unwrap().push(AdminAccount {
            id: id.clone(),
            email: Email::parse(email).unwrap(),
            password_hash: PasswordHash::from_encoded("x"),
            lockout: LockoutState::clear(),
            role,
        });
        id
    }

    fn service(
        admins: Arc<InMemoryAdmins>,
        identities: Arc<InMemoryIdentities>,
        identity: OAuthIdentity,
    ) -> (OAuthLoginService, Arc<InMemoryPending>) {
        let pending = Arc::new(InMemoryPending::default());
        let provider = Arc::new(FakeProvider {
            id: ProviderId::parse("google").unwrap(),
            identity,
            last_request: Mutex::new(None),
        });
        let svc = OAuthLoginService::new(
            vec![provider],
            pending.clone(),
            identities,
            admins,
            Arc::new(FixedSecrets),
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            "https://admin.example.com",
        );
        (svc, pending)
    }

    fn identity(email: &str) -> OAuthIdentity {
        OAuthIdentity {
            provider: ProviderId::parse("google").unwrap(),
            subject: "sub-1".to_string(),
            email: email.to_string(),
        }
    }

    #[tokio::test]
    async fn begin_persists_pending_and_builds_url() {
        let (svc, pending) = service(
            Arc::new(InMemoryAdmins::default()),
            Arc::new(InMemoryIdentities::default()),
            identity("admin@example.com"),
        );
        let outcome = svc.begin("google").await.unwrap();
        assert!(outcome.authorize_url.contains("state=state-123"));
        assert!(outcome.authorize_url.contains("challenge=challenge-xyz"));
        // The pending row exists, keyed by state, with the callback URL.
        let stored = pending
            .rows
            .lock()
            .unwrap()
            .get("state-123")
            .cloned()
            .unwrap();
        assert_eq!(
            stored.redirect_uri,
            "https://admin.example.com/auth/oauth/google/callback"
        );
        assert_eq!(stored.code_verifier, "verifier-xyz");
    }

    #[tokio::test]
    async fn begin_rejects_an_unknown_provider() {
        let (svc, _) = service(
            Arc::new(InMemoryAdmins::default()),
            Arc::new(InMemoryIdentities::default()),
            identity("admin@example.com"),
        );
        assert!(matches!(
            svc.begin("facebook").await,
            Err(OAuthError::Config(_))
        ));
    }

    #[tokio::test]
    async fn complete_links_by_email_then_authenticates() {
        let admins = Arc::new(InMemoryAdmins::default());
        let admin_id = seed_admin(&admins, "admin@example.com", Role::admin());
        let identities = Arc::new(InMemoryIdentities::default());
        let (svc, _) = service(admins, identities.clone(), identity("admin@example.com"));

        svc.begin("google").await.unwrap();
        let authenticated = svc.complete("state-123", "the-code").await.unwrap();
        assert_eq!(authenticated.id, admin_id);
        assert_eq!(authenticated.role, Role::admin());

        // The identity is now linked for next time.
        assert_eq!(
            identities
                .find_admin(&ProviderId::parse("google").unwrap(), "sub-1")
                .await
                .unwrap(),
            Some(admin_id)
        );
    }

    #[tokio::test]
    async fn complete_rejects_a_replayed_state() {
        let admins = Arc::new(InMemoryAdmins::default());
        seed_admin(&admins, "admin@example.com", Role::admin());
        let (svc, _) = service(
            admins,
            Arc::new(InMemoryIdentities::default()),
            identity("admin@example.com"),
        );

        svc.begin("google").await.unwrap();
        svc.complete("state-123", "the-code").await.unwrap();
        // The state was consumed; a second use finds nothing.
        assert!(matches!(
            svc.complete("state-123", "the-code").await,
            Err(OAuthError::InvalidState)
        ));
    }

    #[tokio::test]
    async fn complete_refuses_an_identity_with_no_matching_admin() {
        let (svc, _) = service(
            Arc::new(InMemoryAdmins::default()),
            Arc::new(InMemoryIdentities::default()),
            identity("stranger@example.com"),
        );
        svc.begin("google").await.unwrap();
        assert!(matches!(
            svc.complete("state-123", "the-code").await,
            Err(OAuthError::NoLinkedAccount)
        ));
    }

    #[tokio::test]
    async fn complete_rejects_an_unknown_state() {
        let (svc, _) = service(
            Arc::new(InMemoryAdmins::default()),
            Arc::new(InMemoryIdentities::default()),
            identity("admin@example.com"),
        );
        assert!(matches!(
            svc.complete("never-issued", "the-code").await,
            Err(OAuthError::InvalidState)
        ));
    }
}
