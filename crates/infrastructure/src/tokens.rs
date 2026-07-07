//! CSPRNG adapter implementing the [`SessionTokenGenerator`] port.
//!
//! Reuses the OS-backed RNG the `argon2` crate already depends on (the same
//! one [`crate::password`] uses to generate salts), so this adapter adds no
//! new dependency for randomness.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use domain::{CsrfToken, SessionToken, SessionTokenGenerator};

/// Bytes of entropy per generated token: 256 bits, encoded as 64 hex chars.
const TOKEN_BYTES: usize = 32;

/// A [`SessionTokenGenerator`] backed by the operating system's CSPRNG.
#[derive(Debug, Default, Clone, Copy)]
pub struct SecureRandomTokens;

impl SessionTokenGenerator for SecureRandomTokens {
    fn generate_session_token(&self) -> SessionToken {
        SessionToken::from_raw(random_hex())
    }

    fn generate_csrf_token(&self) -> CsrfToken {
        CsrfToken::from_raw(random_hex())
    }
}

/// `TOKEN_BYTES` of CSPRNG output, lower-case hex encoded.
fn random_hex() -> String {
    let mut buf = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut buf);
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in buf {
        use std::fmt::Write;
        write!(out, "{byte:02x}").expect("writing to a String cannot fail");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_unique() {
        let gen = SecureRandomTokens;
        let a = gen.generate_session_token();
        let b = gen.generate_session_token();
        assert_eq!(a.as_str().len(), 64);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a.as_str(), b.as_str());

        let csrf_a = gen.generate_csrf_token();
        let csrf_b = gen.generate_csrf_token();
        assert_eq!(csrf_a.as_str().len(), 64);
        assert_ne!(csrf_a.as_str(), csrf_b.as_str());
    }
}
