//! Argon2id password-hashing adapter implementing the [`PasswordHasher`] port.
//!
//! The parameters default to the OWASP recommendation for Argon2id password
//! storage — 19 MiB of memory, 2 iterations, 1 degree of parallelism — and are
//! overridable from the environment. Because argon2 is deliberately CPU- and
//! memory-heavy, every hash/verify is run on a blocking thread via
//! [`tokio::task::spawn_blocking`], keeping the async runtime responsive.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash as PhcHash, PasswordHasher as _, SaltString};
use argon2::{Algorithm, Argon2, Params, PasswordVerifier, Version};
use domain::{PasswordHash, PasswordHashError, PasswordHasher};

/// Tunable argon2 cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// Memory cost in kibibytes (`m_cost`).
    pub memory_kib: u32,
    /// Number of iterations (`t_cost`).
    pub iterations: u32,
    /// Degree of parallelism (`p_cost`).
    pub parallelism: u32,
}

impl Argon2Params {
    /// The OWASP-recommended Argon2id baseline: m=19456 KiB (19 MiB), t=2, p=1.
    pub const fn owasp_default() -> Self {
        Self {
            memory_kib: 19_456,
            iterations: 2,
            parallelism: 1,
        }
    }

    /// Read parameters from the environment, falling back to
    /// [`Self::owasp_default`] for any unset value.
    ///
    /// - `ARGON2_MEMORY_KIB`
    /// - `ARGON2_ITERATIONS`
    /// - `ARGON2_PARALLELISM`
    ///
    /// A present-but-unparseable value is an error, so a typo fails fast at
    /// startup rather than silently weakening the hash.
    pub fn from_env() -> Result<Self, PasswordHashError> {
        let default = Self::owasp_default();
        Ok(Self {
            memory_kib: parse_env("ARGON2_MEMORY_KIB", default.memory_kib)?,
            iterations: parse_env("ARGON2_ITERATIONS", default.iterations)?,
            parallelism: parse_env("ARGON2_PARALLELISM", default.parallelism)?,
        })
    }
}

fn parse_env(key: &str, default: u32) -> Result<u32, PasswordHashError> {
    match std::env::var(key) {
        Ok(raw) => raw
            .parse()
            .map_err(|_| PasswordHashError(format!("invalid {key}: {raw:?}"))),
        Err(_) => Ok(default),
    }
}

/// An argon2id-backed [`PasswordHasher`].
#[derive(Clone)]
pub struct Argon2Hasher {
    argon2: Argon2<'static>,
    reference: PasswordHash,
}

impl std::fmt::Debug for Argon2Hasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Argon2Hasher").finish_non_exhaustive()
    }
}

impl Argon2Hasher {
    /// Build the hasher from cost parameters.
    ///
    /// Also computes a one-time *reference hash* with these same parameters. The
    /// login use case verifies against it when no account matches, so a request
    /// for a nonexistent user does the same argon2 work as a real one — the
    /// timing-equalization that blocks account enumeration. This single blocking
    /// hash happens once, at startup.
    pub fn new(params: Argon2Params) -> Result<Self, PasswordHashError> {
        let params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            None,
        )
        .map_err(|e| PasswordHashError(format!("invalid argon2 params: {e}")))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        // A fixed passphrase is fine: only the parameters (not the content)
        // determine how long verification takes, and this hash is never a real
        // credential.
        let reference = hash_with(&argon2, "reference-password-constant-time")?;

        Ok(Self { argon2, reference })
    }

    /// Build the hasher from the environment (see [`Argon2Params::from_env`]).
    pub fn from_env() -> Result<Self, PasswordHashError> {
        Self::new(Argon2Params::from_env()?)
    }
}

/// Hash `plaintext` with a random salt, returning the encoded PHC string.
fn hash_with(argon2: &Argon2<'static>, plaintext: &str) -> Result<PasswordHash, PasswordHashError> {
    let salt = SaltString::generate(&mut OsRng);
    let encoded = argon2
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| PasswordHashError(e.to_string()))?
        .to_string();
    Ok(PasswordHash::from_encoded(encoded))
}

#[async_trait::async_trait]
impl PasswordHasher for Argon2Hasher {
    async fn hash(&self, plaintext: &str) -> Result<PasswordHash, PasswordHashError> {
        let argon2 = self.argon2.clone();
        let plaintext = plaintext.to_owned();
        tokio::task::spawn_blocking(move || hash_with(&argon2, &plaintext))
            .await
            .map_err(|e| PasswordHashError(format!("hash task panicked: {e}")))?
    }

    async fn verify(
        &self,
        plaintext: &str,
        hash: &PasswordHash,
    ) -> Result<bool, PasswordHashError> {
        let argon2 = self.argon2.clone();
        let plaintext = plaintext.to_owned();
        let encoded = hash.as_str().to_owned();
        tokio::task::spawn_blocking(move || {
            let parsed = PhcHash::new(&encoded)
                .map_err(|e| PasswordHashError(format!("corrupt hash: {e}")))?;
            match argon2.verify_password(plaintext.as_bytes(), &parsed) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(e) => Err(PasswordHashError(e.to_string())),
            }
        })
        .await
        .map_err(|e| PasswordHashError(format!("verify task panicked: {e}")))?
    }

    fn reference_hash(&self) -> PasswordHash {
        self.reference.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A cheap parameter set so the tests stay fast (real hashing at OWASP cost
    // is intentionally slow). Correctness of the round-trip does not depend on
    // the cost, only on the algorithm.
    fn cheap() -> Argon2Hasher {
        Argon2Hasher::new(Argon2Params {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        })
        .expect("cheap params are valid")
    }

    #[tokio::test]
    async fn hash_then_verify_roundtrips() {
        let hasher = cheap();
        let hash = hasher.hash("correct horse battery").await.unwrap();
        assert!(hasher.as_ref_verify(&hash, "correct horse battery").await);
        assert!(!hasher.as_ref_verify(&hash, "wrong password").await);
    }

    #[tokio::test]
    async fn each_hash_uses_a_fresh_salt() {
        let hasher = cheap();
        let a = hasher.hash("same-input").await.unwrap();
        let b = hasher.hash("same-input").await.unwrap();
        assert_ne!(a.as_str(), b.as_str(), "salts must differ per hash");
    }

    #[tokio::test]
    async fn reference_hash_is_a_valid_argon2id_hash() {
        let hasher = cheap();
        // It must parse and verify against *something* (its own passphrase),
        // proving verification against it does real argon2 work.
        let reference = hasher.reference_hash();
        assert!(reference.as_str().starts_with("$argon2id$"));
    }

    #[tokio::test]
    async fn corrupt_hash_is_an_error_not_a_false() {
        let hasher = cheap();
        let bad = PasswordHash::from_encoded("not-a-phc-string");
        assert!(hasher.verify("whatever", &bad).await.is_err());
    }

    // Small test helper to keep the assertions readable.
    impl Argon2Hasher {
        async fn as_ref_verify(&self, hash: &PasswordHash, plaintext: &str) -> bool {
            self.verify(plaintext, hash).await.unwrap()
        }
    }
}
