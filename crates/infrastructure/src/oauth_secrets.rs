//! CSPRNG adapter implementing the [`OAuthSecretGenerator`] port: `state`,
//! `nonce`, and the PKCE verifier/`S256`-challenge pair.
//!
//! Reuses the OS RNG the `argon2` crate already brings in (the same one
//! [`crate::tokens`] uses), and computes the PKCE challenge with `sha2` —
//! `base64url(sha256(verifier))`, no padding, per RFC 7636.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use domain::{OAuthSecretGenerator, PkcePair};
use sha2::{Digest, Sha256};

/// Bytes of entropy per generated secret: 256 bits.
const SECRET_BYTES: usize = 32;

/// An [`OAuthSecretGenerator`] backed by the operating system's CSPRNG.
#[derive(Debug, Default, Clone, Copy)]
pub struct OAuthSecrets;

impl OAuthSecretGenerator for OAuthSecrets {
    fn state(&self) -> String {
        random_url_safe()
    }

    fn nonce(&self) -> String {
        random_url_safe()
    }

    fn pkce(&self) -> PkcePair {
        // The verifier is itself a URL-safe base64 string (a valid RFC 7636
        // verifier: 43 chars from the unreserved set). The challenge is the
        // base64url of its SHA-256 digest.
        let verifier = random_url_safe();
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        PkcePair::new(verifier, challenge)
    }
}

/// `SECRET_BYTES` of CSPRNG output, URL-safe base64 without padding.
fn random_url_safe() -> String {
    let mut buf = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_url_safe_and_unique() {
        let gen = OAuthSecrets;
        let a = gen.state();
        let b = gen.state();
        assert_ne!(a, b);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn pkce_challenge_is_the_s256_of_the_verifier() {
        let pair = OAuthSecrets.pkce();
        // Independently recompute the expected challenge from the verifier.
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pair.verifier().as_bytes()));
        assert_eq!(pair.challenge(), expected);
        // A SHA-256 digest is 32 bytes → 43 base64url chars, no padding.
        assert_eq!(pair.challenge().len(), 43);
        assert!(!pair.challenge().contains('='));
    }
}
