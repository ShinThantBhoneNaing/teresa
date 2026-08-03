//! Password hashing.
//!
//! Argon2id with the RFC 9106 low-memory parameters: 19 MiB, 2 passes, 1 lane.
//! Verification therefore costs tens of milliseconds of CPU and 19 MiB of
//! memory per attempt — cheap enough for a login, ruinous for a GPU cracking
//! rig, which is the entire point.
//!
//! Hashes are stored in PHC string format, which records the algorithm and
//! parameters alongside the salt and digest. Raising the parameters later
//! changes only new hashes; old ones keep verifying under the parameters they
//! were minted with.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::{Result, ServerError};

/// The hasher, holding tuned parameters.
///
/// One instance is built at startup and shared; [`Argon2`] is stateless after
/// construction, so sharing is free.
#[derive(Clone)]
pub struct PasswordHasherService {
    argon2: Argon2<'static>,
}

impl std::fmt::Debug for PasswordHasherService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Argon2 does not implement Debug, and its parameters are not secret,
        // but printing them here would only invite log noise.
        f.debug_struct("PasswordHasherService")
            .finish_non_exhaustive()
    }
}

impl PasswordHasherService {
    /// Builds the hasher with the RFC 9106 second recommended parameter set.
    ///
    /// # Panics
    /// Panics only if the hard-coded parameters are invalid, which a unit test
    /// makes impossible to ship.
    #[must_use]
    pub fn new() -> Self {
        let params = Params::new(
            19 * 1024, // memory, KiB
            2,         // passes
            1,         // lanes
            None,      // default 32-byte output
        )
        .expect("hard-coded Argon2 parameters are valid");

        Self {
            argon2: Argon2::new(Algorithm::Argon2id, Version::V0x13, params),
        }
    }

    /// Hashes a password with a fresh random salt.
    ///
    /// # Errors
    /// Returns [`ServerError::PasswordHash`] if hashing fails, which in
    /// practice means the process is out of memory.
    pub fn hash(&self, password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        self.argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| ServerError::PasswordHash)
    }

    /// Verifies a password against a stored PHC hash.
    ///
    /// A mismatch and a corrupt hash both return `false` rather than distinct
    /// errors: the caller's next move — refuse the login — is the same, and
    /// distinguishing them for the client would leak which usernames exist.
    #[must_use]
    pub fn verify(&self, password: &str, stored_hash: &str) -> bool {
        PasswordHash::new(stored_hash).is_ok_and(|parsed| {
            self.argon2
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    }
}

impl Default for PasswordHasherService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let hasher = PasswordHasherService::new();
        let hash = hasher.hash("correct horse battery staple").unwrap();
        assert!(hasher.verify("correct horse battery staple", &hash));
    }

    #[test]
    fn the_wrong_password_does_not_verify() {
        let hasher = PasswordHasherService::new();
        let hash = hasher.hash("correct horse battery staple").unwrap();
        assert!(!hasher.verify("incorrect horse", &hash));
    }

    #[test]
    fn hashes_are_salted() {
        let hasher = PasswordHasherService::new();
        let a = hasher.hash("same password").unwrap();
        let b = hasher.hash("same password").unwrap();
        assert_ne!(a, b, "two hashes of one password must differ by salt");
    }

    #[test]
    fn hashes_are_argon2id_phc_strings() {
        let hasher = PasswordHasherService::new();
        let hash = hasher.hash("whatever suits").unwrap();
        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "got {hash}"
        );
    }

    #[test]
    fn plaintext_never_appears_in_the_hash() {
        let hasher = PasswordHasherService::new();
        let hash = hasher.hash("hunter2hunter2").unwrap();
        assert!(!hash.contains("hunter2"));
    }

    #[test]
    fn a_corrupt_stored_hash_refuses_rather_than_panics() {
        let hasher = PasswordHasherService::new();
        assert!(!hasher.verify("anything at all", "not a phc string"));
        assert!(!hasher.verify("anything at all", ""));
    }

    #[test]
    fn unicode_passwords_round_trip() {
        let hasher = PasswordHasherService::new();
        let hash = hasher.hash("пароль-🔥-passphrase").unwrap();
        assert!(hasher.verify("пароль-🔥-passphrase", &hash));
        assert!(!hasher.verify("пароль-🔥-passphrasE", &hash));
    }
}
