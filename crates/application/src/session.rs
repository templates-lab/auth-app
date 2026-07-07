//! Session use cases: issue a session after login, authenticate an inbound
//! session token on every subsequent request, verify CSRF on mutations, rotate
//! a session without a fresh login, and revoke one on logout.
//!
//! [`SessionService`] holds no knowledge of HTTP or storage — the composition
//! root decides which adapters back the [`SessionRepository`] and
//! [`SessionTokenGenerator`] ports.

use std::sync::Arc;
use std::time::SystemTime;

use domain::{
    AdminId, Clock, CsrfToken, Session, SessionPolicy, SessionRepository, SessionToken,
    SessionTokenGenerator,
};

/// A freshly issued session, as returned to the delivery layer so it can set
/// the session and CSRF cookies.
#[derive(Debug, Clone)]
pub struct IssuedSession {
    /// The bearer token to write into the (`HttpOnly`) session cookie.
    pub token: SessionToken,
    /// The CSRF token to write into the client-readable CSRF cookie.
    pub csrf_token: CsrfToken,
    /// The absolute deadline, so the delivery layer can cap the cookie's
    /// `Max-Age` at the same ceiling the server enforces.
    pub absolute_expires_at: SystemTime,
}

/// The result of successfully authenticating an inbound session token.
#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    /// The admin this session belongs to.
    pub admin_id: AdminId,
    /// The session's CSRF token, to check against the request header on
    /// mutating requests.
    pub csrf_token: CsrfToken,
}

/// Why a session use case was rejected.
#[derive(Debug)]
pub enum SessionError {
    /// No session exists for the given token (never issued, already revoked,
    /// or already deleted after expiring).
    NotFound,
    /// A session exists but is past its idle or absolute deadline.
    Expired,
    /// The request was a mutation but the CSRF header was missing or did not
    /// match the session's CSRF token.
    CsrfMismatch,
    /// An internal failure (storage). Never leaks specifics to the caller.
    Internal(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no such session"),
            Self::Expired => f.write_str("session expired"),
            Self::CsrfMismatch => f.write_str("csrf token missing or mismatched"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for SessionError {}

fn internal(e: domain::RepositoryError) -> SessionError {
    SessionError::Internal(e.to_string())
}

/// Application service for the session lifecycle.
///
/// Cloneable (everything inside is an `Arc` or `Copy`), so the delivery layer
/// can hold it as router/middleware state.
#[derive(Clone)]
pub struct SessionService {
    sessions: Arc<dyn SessionRepository>,
    tokens: Arc<dyn SessionTokenGenerator>,
    clock: Arc<dyn Clock>,
    policy: SessionPolicy,
}

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionService")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl SessionService {
    /// Assemble the service from its ports and the expiration policy.
    pub fn new(
        sessions: Arc<dyn SessionRepository>,
        tokens: Arc<dyn SessionTokenGenerator>,
        clock: Arc<dyn Clock>,
        policy: SessionPolicy,
    ) -> Self {
        Self {
            sessions,
            tokens,
            clock,
            policy,
        }
    }

    /// Issue a brand-new session for an already-authenticated admin.
    ///
    /// Called right after [`crate::LoginService::login`] succeeds — every
    /// login therefore gets a fresh, unguessable token pair rather than
    /// reusing anything from a prior session, which is the "rotate on login"
    /// requirement satisfied by construction.
    pub async fn start(&self, admin_id: AdminId) -> Result<IssuedSession, SessionError> {
        let now = self.clock.now();
        let token = self.tokens.generate_session_token();
        let csrf_token = self.tokens.generate_csrf_token();
        let absolute_expires_at = now + self.policy.absolute_timeout;

        let session = Session {
            token: token.clone(),
            admin_id,
            csrf_token: csrf_token.clone(),
            created_at: now,
            absolute_expires_at,
            last_seen_at: now,
        };
        self.sessions.insert(&session).await.map_err(internal)?;

        Ok(IssuedSession {
            token,
            csrf_token,
            absolute_expires_at,
        })
    }

    /// Authenticate an inbound session token: reject an unknown or expired
    /// session, else slide the idle window forward and return the session's
    /// identity for the delivery layer to act on.
    pub async fn authenticate(
        &self,
        token: &SessionToken,
    ) -> Result<AuthenticatedSession, SessionError> {
        let now = self.clock.now();
        let session = self
            .sessions
            .find(token)
            .await
            .map_err(internal)?
            .ok_or(SessionError::NotFound)?;

        if self.policy.is_expired(&session, now) {
            // Best-effort cleanup; the expiry verdict stands either way.
            let _ = self.sessions.delete(token).await;
            return Err(SessionError::Expired);
        }

        self.sessions.touch(token, now).await.map_err(internal)?;

        Ok(AuthenticatedSession {
            admin_id: session.admin_id,
            csrf_token: session.csrf_token,
        })
    }

    /// Verify a mutating request's CSRF header against the authenticated
    /// session. Call this for every state-changing request, after
    /// [`Self::authenticate`] has confirmed the session itself is live.
    pub fn verify_csrf(
        &self,
        session: &AuthenticatedSession,
        header_value: Option<&str>,
    ) -> Result<(), SessionError> {
        match header_value {
            Some(candidate) if session.csrf_token.verify(candidate) => Ok(()),
            _ => Err(SessionError::CsrfMismatch),
        }
    }

    /// Rotate a session: issue a fresh token/CSRF pair for the same admin and
    /// revoke the old one, without requiring a fresh login.
    ///
    /// Callers invoke this whenever a session's privilege level changes (for
    /// example, a future step-up to a higher role) so that a token captured
    /// before the change cannot ride along after it.
    pub async fn rotate(&self, old_token: &SessionToken) -> Result<IssuedSession, SessionError> {
        let authenticated = self.authenticate(old_token).await?;
        self.sessions.delete(old_token).await.map_err(internal)?;
        self.start(authenticated.admin_id).await
    }

    /// Revoke a session server-side (logout). Idempotent: revoking a token
    /// that is already gone is not an error.
    pub async fn revoke(&self, token: &SessionToken) -> Result<(), SessionError> {
        self.sessions.delete(token).await.map_err(internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use domain::RepositoryError;

    #[derive(Default)]
    struct InMemorySessions {
        rows: Mutex<HashMap<String, Session>>,
    }

    #[async_trait::async_trait]
    impl SessionRepository for InMemorySessions {
        async fn insert(&self, session: &Session) -> Result<(), RepositoryError> {
            self.rows
                .lock()
                .unwrap()
                .insert(session.token.as_str().to_string(), session.clone());
            Ok(())
        }
        async fn find(&self, token: &SessionToken) -> Result<Option<Session>, RepositoryError> {
            Ok(self.rows.lock().unwrap().get(token.as_str()).cloned())
        }
        async fn touch(
            &self,
            token: &SessionToken,
            last_seen_at: SystemTime,
        ) -> Result<(), RepositoryError> {
            if let Some(row) = self.rows.lock().unwrap().get_mut(token.as_str()) {
                row.last_seen_at = last_seen_at;
            }
            Ok(())
        }
        async fn delete(&self, token: &SessionToken) -> Result<(), RepositoryError> {
            self.rows.lock().unwrap().remove(token.as_str());
            Ok(())
        }
    }

    // A generator that hands out sequential, predictable tokens so tests can
    // assert rotation actually changes them.
    #[derive(Default)]
    struct SequentialTokens {
        next: Mutex<u64>,
    }

    impl SessionTokenGenerator for SequentialTokens {
        fn generate_session_token(&self) -> SessionToken {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            SessionToken::from_raw(format!("session-{n}"))
        }
        fn generate_csrf_token(&self) -> CsrfToken {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            CsrfToken::from_raw(format!("csrf-{n}"))
        }
    }

    struct FixedClock(Mutex<SystemTime>);
    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().unwrap()
        }
    }

    fn service(now: SystemTime, policy: SessionPolicy) -> (SessionService, Arc<InMemorySessions>) {
        let sessions = Arc::new(InMemorySessions::default());
        let svc = SessionService::new(
            sessions.clone(),
            Arc::new(SequentialTokens::default()),
            Arc::new(FixedClock(Mutex::new(now))),
            policy,
        );
        (svc, sessions)
    }

    #[tokio::test]
    async fn start_then_authenticate_round_trips() {
        let (svc, _) = service(SystemTime::UNIX_EPOCH, SessionPolicy::recommended());
        let issued = svc.start(AdminId::new("admin-1")).await.unwrap();

        let authenticated = svc.authenticate(&issued.token).await.unwrap();
        assert_eq!(authenticated.admin_id, AdminId::new("admin-1"));
        assert!(authenticated.csrf_token.verify(issued.csrf_token.as_str()));
    }

    #[tokio::test]
    async fn unknown_token_is_not_found() {
        let (svc, _) = service(SystemTime::UNIX_EPOCH, SessionPolicy::recommended());
        let err = svc
            .authenticate(&SessionToken::from_raw("nope"))
            .await
            .unwrap_err();
        assert!(matches!(err, SessionError::NotFound));
    }

    #[tokio::test]
    async fn expired_session_is_rejected_and_cleaned_up() {
        let now = SystemTime::UNIX_EPOCH;
        let policy = SessionPolicy {
            idle_timeout: Duration::from_secs(10),
            absolute_timeout: Duration::from_secs(3600),
        };
        let sessions = Arc::new(InMemorySessions::default());
        let clock = Arc::new(FixedClock(Mutex::new(now)));
        let svc = SessionService::new(
            sessions.clone(),
            Arc::new(SequentialTokens::default()),
            clock.clone(),
            policy,
        );

        let issued = svc.start(AdminId::new("admin-1")).await.unwrap();

        // Advance past the idle timeout.
        *clock.0.lock().unwrap() = now + Duration::from_secs(11);
        let err = svc.authenticate(&issued.token).await.unwrap_err();
        assert!(matches!(err, SessionError::Expired));

        // The expired session was removed as a side effect.
        assert!(sessions.find(&issued.token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn csrf_verification_requires_matching_header() {
        let (svc, _) = service(SystemTime::UNIX_EPOCH, SessionPolicy::recommended());
        let issued = svc.start(AdminId::new("admin-1")).await.unwrap();
        let authenticated = svc.authenticate(&issued.token).await.unwrap();

        assert!(svc
            .verify_csrf(&authenticated, Some(issued.csrf_token.as_str()))
            .is_ok());
        assert!(matches!(
            svc.verify_csrf(&authenticated, Some("wrong")),
            Err(SessionError::CsrfMismatch)
        ));
        assert!(matches!(
            svc.verify_csrf(&authenticated, None),
            Err(SessionError::CsrfMismatch)
        ));
    }

    #[tokio::test]
    async fn rotate_issues_a_new_token_and_kills_the_old_one() {
        let (svc, sessions) = service(SystemTime::UNIX_EPOCH, SessionPolicy::recommended());
        let issued = svc.start(AdminId::new("admin-1")).await.unwrap();

        let rotated = svc.rotate(&issued.token).await.unwrap();
        assert_ne!(rotated.token, issued.token);

        // The old token is dead...
        assert!(sessions.find(&issued.token).await.unwrap().is_none());
        // ...and the new one authenticates as the same admin.
        let authenticated = svc.authenticate(&rotated.token).await.unwrap();
        assert_eq!(authenticated.admin_id, AdminId::new("admin-1"));
    }

    #[tokio::test]
    async fn revoke_deletes_the_session() {
        let (svc, sessions) = service(SystemTime::UNIX_EPOCH, SessionPolicy::recommended());
        let issued = svc.start(AdminId::new("admin-1")).await.unwrap();

        svc.revoke(&issued.token).await.unwrap();
        assert!(sessions.find(&issued.token).await.unwrap().is_none());
        assert!(matches!(
            svc.authenticate(&issued.token).await.unwrap_err(),
            SessionError::NotFound
        ));
    }
}
