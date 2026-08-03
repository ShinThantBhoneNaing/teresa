//! Session tokens.
//!
//! A token is 32 bytes from the OS CSPRNG, handed to the client as base64url.
//! The database stores only its SHA-256: a stolen database yields no usable
//! sessions, and lookup is still a single indexed equality. Argon2 would be
//! wasted here — it exists to slow down guessing of *low-entropy* secrets, and
//! a 256-bit random token cannot be guessed at any speed.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

/// Bytes of entropy in a token.
const TOKEN_BYTES: usize = 32;

/// A freshly minted session token: the secret for the client and the hash for
/// the database, produced together so no code path can store the wrong one.
#[derive(Clone)]
pub struct MintedToken {
    /// The secret, base64url. Sent to the client once and never stored.
    pub secret: String,
    /// Hex SHA-256 of the secret. This is what the sessions table keeps.
    pub hash: String,
}

impl std::fmt::Debug for MintedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret must survive neither logs nor panic messages.
        f.debug_struct("MintedToken")
            .field("hash", &self.hash)
            .finish_non_exhaustive()
    }
}

/// Mints a new session token.
#[must_use]
pub fn mint() -> MintedToken {
    use argon2::password_hash::rand_core::{OsRng, RngCore};

    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);

    let secret = URL_SAFE_NO_PAD.encode(bytes);
    MintedToken {
        hash: hash(&secret),
        secret,
    }
}

/// Hashes a presented token for lookup.
///
/// The input is the base64 text exactly as the client presented it, not the
/// decoded bytes — decoding first would make two distinct client strings that
/// decode identically collide, and gains nothing.
#[must_use]
pub fn hash(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    hex_encode(&digest)
}

/// Lowercase hex, written out here to avoid a dependency for sixteen lines.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_tokens_are_unique() {
        let a = mint();
        let b = mint();
        assert_ne!(a.secret, b.secret);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn the_hash_matches_a_rehash_of_the_secret() {
        let token = mint();
        assert_eq!(hash(&token.secret), token.hash);
    }

    #[test]
    fn secrets_are_url_safe_base64_of_32_bytes() {
        let token = mint();
        let decoded = URL_SAFE_NO_PAD
            .decode(&token.secret)
            .expect("valid base64url");
        assert_eq!(decoded.len(), TOKEN_BYTES);
        assert!(!token.secret.contains('+') && !token.secret.contains('/'));
    }

    #[test]
    fn hashes_are_hex_sha256() {
        let token = mint();
        assert_eq!(token.hash.len(), 64);
        assert!(token
            .hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn a_wrong_token_hashes_differently() {
        let token = mint();
        assert_ne!(hash("forged-token"), token.hash);
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let token = mint();
        let debug = format!("{token:?}");
        assert!(!debug.contains(&token.secret));
        assert!(debug.contains(&token.hash));
    }

    #[test]
    fn hex_encoding_is_correct() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x1a]), "00ff1a");
        assert_eq!(hex_encode(&[]), "");
    }
}
