//! Administrator authentication use cases.
//!
//! [`LoginService`] verifies an email/password pair with three defenses wired
//! together: argon2 verification (through the [`PasswordHasher`] port), a
//! constant-work path so a nonexistent account cannot be distinguished from a
//! wrong password by timing or response, and progressive lockout at both the
//! account and the client-IP level. [`BootstrapService`] seeds the first
//! administrator from configuration so no password is ever committed to the
//! repository.
//!
//! Both services receive domain ports (as `Arc<dyn _>`) and hold no knowledge of
//! HTTP or storage — the composition root decides which adapters back them.

use std::sync::Arc;

use domain::{
    AdminId, AdminRepository, Clock, Email, IpLockoutStore, LockoutPolicy, NewAdmin,
    PasswordHasher, PasswordPolicy, PasswordPolicyError, Role,
};

/// A single login attempt: the submitted credentials plus the client's address.
///
/// The `client_ip` is the identity used for IP-level lockout. Behind a reverse
/// proxy the delivery layer resolves it from the forwarded headers before
/// building this request.
#[derive(Debug, Clone)]
pub struct LoginRequest {
    /// The submitted email (unparsed — the service normalizes it).
    pub email: String,
    /// The submitted plaintext password.
    pub password: String,
    /// The client's IP address, for per-IP throttling.
    pub client_ip: String,
}

/// The outcome of a rejected login, kept deliberately coarse.
///
/// [`Self::InvalidCredentials`] is returned identically for a nonexistent
/// account and a wrong password — the two must be indistinguishable. Lockout is
/// a *separate* axis ([`Self::TooManyAttempts`]); reporting it does not leak
/// account existence because an IP is throttled even for emails that were never
/// registered.
#[derive(Debug)]
pub enum LoginError {
    /// Wrong password, or no such account — indistinguishable by design.
    InvalidCredentials,
    /// The account or the client IP is currently locked out.
    TooManyAttempts {
        /// Seconds the caller should wait before retrying, if known.
        retry_after_secs: Option<u64>,
    },
    /// An internal failure (hashing or storage). Never leaks specifics to the
    /// caller; the delivery layer maps it to a 500.
    Internal(String),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCredentials => f.write_str("invalid credentials"),
            Self::TooManyAttempts { .. } => f.write_str("too many attempts"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for LoginError {}

/// The identity of a successfully authenticated administrator: enough for the
/// delivery layer to issue a session ([`crate::SessionService::start`]) and
/// record the login to the audit trail, without re-fetching the account.
#[derive(Debug, Clone)]
pub struct AuthenticatedAdmin {
    /// The authenticated administrator's id.
    pub id: AdminId,
    /// The administrator's role, for the session the delivery layer issues
    /// next and for the RBAC middleware that later reads it back.
    pub role: Role,
}

/// Application service for the admin login use case.
///
/// Cloneable (everything inside is an `Arc` or `Copy`), so the delivery layer
/// can hold it as router state.
#[derive(Clone)]
pub struct LoginService {
    repo: Arc<dyn AdminRepository>,
    ip_lockouts: Arc<dyn IpLockoutStore>,
    hasher: Arc<dyn PasswordHasher>,
    clock: Arc<dyn Clock>,
    policy: LockoutPolicy,
}

impl std::fmt::Debug for LoginService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginService")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl LoginService {
    /// Assemble the service from its ports and the lockout policy.
    pub fn new(
        repo: Arc<dyn AdminRepository>,
        ip_lockouts: Arc<dyn IpLockoutStore>,
        hasher: Arc<dyn PasswordHasher>,
        clock: Arc<dyn Clock>,
        policy: LockoutPolicy,
    ) -> Self {
        Self {
            repo,
            ip_lockouts,
            hasher,
            clock,
            policy,
        }
    }

    /// Attempt a login, returning the authenticated [`AdminId`] on success.
    ///
    /// The flow is ordered to preserve the two security properties:
    ///
    /// 1. **IP lockout first.** A throttled IP is rejected before any account
    ///    lookup, so a flood cannot even probe for accounts.
    /// 2. **Constant work.** Whether or not the email matches an account, the
    ///    service runs exactly one argon2 verification — against the real hash
    ///    or the hasher's reference hash — so response and timing are identical
    ///    for "no such user" and "wrong password".
    /// 3. **Record failures on both axes.** A failure bumps the IP counter
    ///    always and the account counter when the account exists; a success
    ///    clears both.
    pub async fn login(&self, request: LoginRequest) -> Result<AuthenticatedAdmin, LoginError> {
        let now = self.clock.now();
        let ip = request.client_ip.as_str();

        // 1. IP-level lockout short-circuit.
        let ip_state = self
            .ip_lockouts
            .get(ip)
            .await
            .map_err(|e| LoginError::Internal(e.to_string()))?;
        if self.policy.is_locked(&ip_state, now) {
            return Err(self.too_many_attempts(&ip_state, now));
        }

        // Parse the email. An unparseable email cannot match any account, but we
        // still perform the reference verification below so timing does not
        // reveal that the input never reached storage.
        let account = match Email::parse(&request.email) {
            Ok(email) => self
                .repo
                .find_by_email(&email)
                .await
                .map_err(|e| LoginError::Internal(e.to_string()))?,
            Err(_) => None,
        };

        // 2. Constant-work verification: real hash if the account exists, else
        // the reference hash. Exactly one argon2 evaluation happens either way.
        let hash = account
            .as_ref()
            .map(|a| a.password_hash.clone())
            .unwrap_or_else(|| self.hasher.reference_hash());
        let verified = self
            .hasher
            .verify(&request.password, &hash)
            .await
            .map_err(|e| LoginError::Internal(e.to_string()))?;

        match account {
            // An account matched.
            Some(account) => {
                // A locked account is rejected even with the right password.
                if self.policy.is_locked(&account.lockout, now) {
                    self.record_ip_failure(ip, &ip_state, now).await?;
                    return Err(self.too_many_attempts(&account.lockout, now));
                }
                if verified {
                    // Success: clear both counters.
                    self.reset_account(&account.id, now).await?;
                    self.reset_ip(ip, now).await?;
                    Ok(AuthenticatedAdmin {
                        id: account.id,
                        role: account.role,
                    })
                } else {
                    // Wrong password: bump the account and IP counters.
                    let next = self.policy.register_failure(&account.lockout, now);
                    self.repo
                        .update_lockout(&account.id, &next)
                        .await
                        .map_err(|e| LoginError::Internal(e.to_string()))?;
                    self.record_ip_failure(ip, &ip_state, now).await?;
                    Err(LoginError::InvalidCredentials)
                }
            }
            // No account: only the IP counter moves. Same error, same work.
            None => {
                self.record_ip_failure(ip, &ip_state, now).await?;
                Err(LoginError::InvalidCredentials)
            }
        }
    }

    /// Build a [`LoginError::TooManyAttempts`] carrying the remaining lock time.
    fn too_many_attempts(
        &self,
        state: &domain::LockoutState,
        now: std::time::SystemTime,
    ) -> LoginError {
        LoginError::TooManyAttempts {
            retry_after_secs: self.policy.retry_after(state, now).map(|d| d.as_secs()),
        }
    }

    /// Persist an IP failure computed from its prior state.
    async fn record_ip_failure(
        &self,
        ip: &str,
        prior: &domain::LockoutState,
        now: std::time::SystemTime,
    ) -> Result<(), LoginError> {
        let next = self.policy.register_failure(prior, now);
        self.ip_lockouts
            .put(ip, &next)
            .await
            .map_err(|e| LoginError::Internal(e.to_string()))
    }

    /// Clear an account's lockout counters after a success.
    async fn reset_account(
        &self,
        id: &AdminId,
        _now: std::time::SystemTime,
    ) -> Result<(), LoginError> {
        self.repo
            .update_lockout(id, &domain::LockoutState::clear())
            .await
            .map_err(|e| LoginError::Internal(e.to_string()))
    }

    /// Clear an IP's lockout counters after a success from it.
    async fn reset_ip(&self, ip: &str, _now: std::time::SystemTime) -> Result<(), LoginError> {
        self.ip_lockouts
            .put(ip, &domain::LockoutState::clear())
            .await
            .map_err(|e| LoginError::Internal(e.to_string()))
    }
}

/// Application service that seeds the first administrator.
///
/// The password arrives from the environment (a secret), never from the
/// repository, satisfying "bootstrap the first admin with no password in the
/// repo". The operation is idempotent-friendly: it refuses to run once any admin
/// exists, so re-invoking the bootstrap command cannot silently reset an
/// account.
#[derive(Clone)]
pub struct BootstrapService {
    repo: Arc<dyn AdminRepository>,
    hasher: Arc<dyn PasswordHasher>,
    policy: PasswordPolicy,
}

impl std::fmt::Debug for BootstrapService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapService")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// The result of a bootstrap attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum BootstrapOutcome {
    /// A new admin was created, with the assigned id.
    Created(AdminId),
    /// An administrator already existed; nothing was changed.
    AlreadyInitialized,
}

/// Why a bootstrap attempt failed.
#[derive(Debug)]
pub enum BootstrapError {
    /// The supplied email was invalid.
    InvalidEmail(domain::EmailError),
    /// The supplied password did not meet the policy.
    WeakPassword(PasswordPolicyError),
    /// A hashing or storage failure.
    Internal(String),
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEmail(e) => write!(f, "invalid admin email: {e}"),
            Self::WeakPassword(e) => write!(f, "{e}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for BootstrapError {}

impl BootstrapService {
    /// Assemble the service from its ports and the password policy.
    pub fn new(
        repo: Arc<dyn AdminRepository>,
        hasher: Arc<dyn PasswordHasher>,
        policy: PasswordPolicy,
    ) -> Self {
        Self {
            repo,
            hasher,
            policy,
        }
    }

    /// Create the first administrator if none exists yet.
    ///
    /// Validates the email and enforces the password policy *before* touching
    /// storage, then no-ops (returning [`BootstrapOutcome::AlreadyInitialized`])
    /// if any admin is already present.
    pub async fn create_first_admin(
        &self,
        email: &str,
        password: &str,
    ) -> Result<BootstrapOutcome, BootstrapError> {
        let email = Email::parse(email).map_err(BootstrapError::InvalidEmail)?;
        self.policy
            .validate(password)
            .map_err(BootstrapError::WeakPassword)?;

        let existing = self
            .repo
            .count()
            .await
            .map_err(|e| BootstrapError::Internal(e.to_string()))?;
        if existing > 0 {
            return Ok(BootstrapOutcome::AlreadyInitialized);
        }

        let password_hash = self
            .hasher
            .hash(password)
            .await
            .map_err(|e| BootstrapError::Internal(e.to_string()))?;
        let id = self
            .repo
            .insert(&NewAdmin {
                email,
                password_hash,
                // The very first administrator is always `admin` — there is
                // no lower-privileged role to choose from before any account
                // exists to grant one.
                role: Role::admin(),
            })
            .await
            .map_err(|e| BootstrapError::Internal(e.to_string()))?;
        Ok(BootstrapOutcome::Created(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    use domain::{AdminAccount, LockoutState, PasswordHash, PasswordHashError, RepositoryError};

    // A hasher whose "hash" is just the plaintext prefixed, so verification is a
    // string compare — enough to exercise the use case without real argon2.
    #[derive(Default)]
    struct FakeHasher;

    #[async_trait::async_trait]
    impl PasswordHasher for FakeHasher {
        async fn hash(&self, plaintext: &str) -> Result<PasswordHash, PasswordHashError> {
            Ok(PasswordHash::from_encoded(format!("hash:{plaintext}")))
        }
        async fn verify(
            &self,
            plaintext: &str,
            hash: &PasswordHash,
        ) -> Result<bool, PasswordHashError> {
            Ok(hash.as_str() == format!("hash:{plaintext}"))
        }
        fn reference_hash(&self) -> PasswordHash {
            PasswordHash::from_encoded("hash:__reference__")
        }
    }

    #[derive(Default)]
    struct InMemoryRepo {
        accounts: Mutex<Vec<AdminAccount>>,
    }

    #[async_trait::async_trait]
    impl AdminRepository for InMemoryRepo {
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
            id: &AdminId,
            state: &LockoutState,
        ) -> Result<(), RepositoryError> {
            for a in self.accounts.lock().unwrap().iter_mut() {
                if &a.id == id {
                    a.lockout = *state;
                }
            }
            Ok(())
        }
        async fn count(&self) -> Result<u64, RepositoryError> {
            Ok(self.accounts.lock().unwrap().len() as u64)
        }
        async fn insert(&self, admin: &NewAdmin) -> Result<AdminId, RepositoryError> {
            let mut accounts = self.accounts.lock().unwrap();
            if accounts.iter().any(|a| a.email == admin.email) {
                return Err(RepositoryError::EmailTaken);
            }
            let id = AdminId::new(format!("id-{}", accounts.len() + 1));
            accounts.push(AdminAccount {
                id: id.clone(),
                email: admin.email.clone(),
                password_hash: admin.password_hash.clone(),
                lockout: LockoutState::clear(),
                role: admin.role.clone(),
            });
            Ok(id)
        }
    }

    #[derive(Default)]
    struct InMemoryIpStore {
        entries: Mutex<HashMap<String, LockoutState>>,
    }

    #[async_trait::async_trait]
    impl IpLockoutStore for InMemoryIpStore {
        async fn get(&self, ip: &str) -> Result<LockoutState, RepositoryError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(ip)
                .copied()
                .unwrap_or_default())
        }
        async fn put(&self, ip: &str, state: &LockoutState) -> Result<(), RepositoryError> {
            self.entries.lock().unwrap().insert(ip.to_string(), *state);
            Ok(())
        }
    }

    // A clock the test can advance by hand.
    struct FixedClock(Mutex<SystemTime>);
    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().unwrap()
        }
    }

    fn seed_admin(repo: &InMemoryRepo, email: &str, password: &str) {
        repo.accounts.lock().unwrap().push(AdminAccount {
            id: AdminId::new("id-1"),
            email: Email::parse(email).unwrap(),
            password_hash: PasswordHash::from_encoded(format!("hash:{password}")),
            lockout: LockoutState::clear(),
            role: Role::admin(),
        });
    }

    fn service(
        repo: Arc<InMemoryRepo>,
        ips: Arc<InMemoryIpStore>,
        clock: Arc<FixedClock>,
        policy: LockoutPolicy,
    ) -> LoginService {
        LoginService::new(repo, ips, Arc::new(FakeHasher), clock, policy)
    }

    fn req(email: &str, password: &str, ip: &str) -> LoginRequest {
        LoginRequest {
            email: email.to_string(),
            password: password.to_string(),
            client_ip: ip.to_string(),
        }
    }

    #[tokio::test]
    async fn correct_password_authenticates() {
        let repo = Arc::new(InMemoryRepo::default());
        seed_admin(&repo, "admin@example.com", "correct horse");
        let clock = Arc::new(FixedClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let svc = service(
            repo,
            Arc::new(InMemoryIpStore::default()),
            clock,
            LockoutPolicy::recommended(),
        );

        let authenticated = svc
            .login(req("admin@example.com", "correct horse", "10.0.0.1"))
            .await
            .unwrap();
        assert_eq!(authenticated.id.as_str(), "id-1");
        assert_eq!(authenticated.role, Role::admin());
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_are_indistinguishable() {
        let repo = Arc::new(InMemoryRepo::default());
        seed_admin(&repo, "admin@example.com", "correct horse");
        let clock = Arc::new(FixedClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let svc = service(
            repo,
            Arc::new(InMemoryIpStore::default()),
            clock,
            LockoutPolicy::recommended(),
        );

        let wrong = svc
            .login(req("admin@example.com", "nope", "10.0.0.1"))
            .await;
        let unknown = svc
            .login(req("ghost@example.com", "nope", "10.0.0.2"))
            .await;
        assert!(matches!(wrong, Err(LoginError::InvalidCredentials)));
        assert!(matches!(unknown, Err(LoginError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn account_locks_after_threshold_then_rejects_even_correct_password() {
        let repo = Arc::new(InMemoryRepo::default());
        seed_admin(&repo, "admin@example.com", "correct horse");
        let clock = Arc::new(FixedClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let policy = LockoutPolicy {
            max_attempts: 3,
            base_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(600),
        };
        // Use a distinct IP per attempt so the IP lock does not mask the account
        // lock we are exercising.
        let svc = service(repo, Arc::new(InMemoryIpStore::default()), clock, policy);

        for i in 0..3 {
            let e = svc
                .login(req("admin@example.com", "wrong", &format!("10.0.0.{i}")))
                .await;
            assert!(matches!(e, Err(LoginError::InvalidCredentials)));
        }
        // Now locked: even the correct password is refused.
        let locked = svc
            .login(req("admin@example.com", "correct horse", "10.0.0.9"))
            .await;
        assert!(matches!(locked, Err(LoginError::TooManyAttempts { .. })));
    }

    #[tokio::test]
    async fn ip_locks_out_across_different_emails() {
        let repo = Arc::new(InMemoryRepo::default());
        let clock = Arc::new(FixedClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let policy = LockoutPolicy {
            max_attempts: 3,
            base_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(600),
        };
        let svc = service(repo, Arc::new(InMemoryIpStore::default()), clock, policy);

        // Three misses from one IP against nonexistent accounts still throttle it.
        for i in 0..3 {
            let _ = svc
                .login(req(&format!("ghost{i}@example.com"), "x", "203.0.113.7"))
                .await;
        }
        let blocked = svc
            .login(req("another@example.com", "x", "203.0.113.7"))
            .await;
        assert!(matches!(blocked, Err(LoginError::TooManyAttempts { .. })));
    }

    #[tokio::test]
    async fn success_after_failures_clears_counters() {
        let repo = Arc::new(InMemoryRepo::default());
        seed_admin(&repo, "admin@example.com", "correct horse");
        let clock = Arc::new(FixedClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let policy = LockoutPolicy {
            max_attempts: 5,
            base_delay: Duration::from_secs(60),
            max_delay: Duration::from_secs(600),
        };
        let ips = Arc::new(InMemoryIpStore::default());
        let svc = service(repo, ips.clone(), clock, policy);

        for _ in 0..2 {
            let _ = svc
                .login(req("admin@example.com", "wrong", "198.51.100.1"))
                .await;
        }
        // A success resets the IP counter to the cleared state.
        svc.login(req("admin@example.com", "correct horse", "198.51.100.1"))
            .await
            .unwrap();
        assert_eq!(
            ips.get("198.51.100.1").await.unwrap(),
            LockoutState::clear()
        );
    }

    #[tokio::test]
    async fn bootstrap_creates_then_is_idempotent_and_enforces_policy() {
        let repo = Arc::new(InMemoryRepo::default());
        let svc = BootstrapService::new(
            repo.clone(),
            Arc::new(FakeHasher),
            PasswordPolicy::recommended(),
        );

        // Weak password rejected before any write.
        assert!(matches!(
            svc.create_first_admin("admin@example.com", "short").await,
            Err(BootstrapError::WeakPassword(_))
        ));
        assert_eq!(repo.count().await.unwrap(), 0);

        // First strong bootstrap creates the admin.
        let outcome = svc
            .create_first_admin("admin@example.com", "Str0ngEnoughPass")
            .await
            .unwrap();
        assert!(matches!(outcome, BootstrapOutcome::Created(_)));

        // A second bootstrap no-ops rather than resetting anything.
        let again = svc
            .create_first_admin("other@example.com", "Str0ngEnoughPass")
            .await
            .unwrap();
        assert_eq!(again, BootstrapOutcome::AlreadyInitialized);
        assert_eq!(repo.count().await.unwrap(), 1);
    }
}
