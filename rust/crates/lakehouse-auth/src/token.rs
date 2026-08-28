//! Opaque token generation and hashing, shared by [`crate::session`] and
//! [`crate::service_token`].

use std::fmt::Write as _;

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::secret::Secret;

/// How many random bytes back an opaque token. 32 bytes (256 bits) from a
/// CSPRNG is far beyond what's brute-forceable — the token's entropy, not
/// this crate's hashing or lookup logic, is what makes an opaque token
/// safe to treat as a password-equivalent secret.
const TOKEN_BYTES: usize = 32;

/// Generate a fresh opaque token: `TOKEN_BYTES` bytes from
/// [`rand::rngs::OsRng`] (via [`rand::thread_rng`], which is seeded from the
/// OS CSPRNG), lower-hex encoded. Collisions are not a practical concern —
/// 256 bits of entropy hex-encoded gives a birthday bound far beyond any
/// realistic number of concurrently issued sessions/service credentials —
/// but callers still enforce uniqueness at the database level
/// (`UNIQUE (token_hash)`) rather than assuming it.
#[must_use]
pub fn generate_opaque_token() -> Secret {
    let mut bytes = [0_u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    Secret::new(out)
}

/// SHA-256 the token's bytes and render the digest as lower-hex. This is
/// what actually gets stored — never the token itself. SHA-256 (not
/// `Argon2`) is the right primitive here specifically because the input is
/// already a full-entropy, unguessable 256-bit value rather than a
/// human-chosen password: there is no "slow the attacker's guessing down"
/// need to serve, only "don't store the bearer-equivalent secret in
/// plaintext", and a fast cryptographic hash is the standard choice for
/// that (mirrors how session/API tokens are hashed in most frameworks).
#[must_use]
pub fn hash_token(token: &Secret) -> String {
    let digest = Sha256::digest(token.expose().as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn generated_tokens_are_64_hex_characters() {
        let token = generate_opaque_token();
        assert_eq!(token.expose().len(), 64);
        assert!(token.expose().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn two_generated_tokens_differ() {
        let a = generate_opaque_token();
        let b = generate_opaque_token();
        assert!(!a.constant_time_eq(&b));
    }

    #[test]
    fn hashing_is_deterministic() {
        let token = Secret::new("fixed-value-for-this-test");
        assert_eq!(hash_token(&token), hash_token(&token));
    }

    #[test]
    fn different_tokens_hash_differently() {
        let a = Secret::new("token-a");
        let b = Secret::new("token-b");
        assert_ne!(hash_token(&a), hash_token(&b));
    }

    #[test]
    fn hash_is_64_hex_characters() {
        let token = generate_opaque_token();
        let hash = hash_token(&token);
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
