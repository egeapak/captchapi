//! One-way hashing of CAPTCHA solutions.
//!
//! Solutions are short (1–20 characters) and drawn from a small alphanumeric
//! alphabet, so a bare digest would be reversible by brute force in
//! milliseconds — a rainbow table over the whole keyspace is cheap to build.
//! Instead the stored value is an HMAC-SHA256 keyed with a server-side secret
//! that never lives in the database: whoever reads the `sessions` table cannot
//! recover the answer without also stealing the key.
//!
//! The session ID is mixed into the message as a per-session salt, so two
//! sessions with the same answer produce different hashes and an attacker
//! cannot group sessions by solution.

use crate::services::hmac::hmac_sha256;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Domain separation for the derived HMAC key, so that the same secret used as
/// `API_KEY_SALT` never produces overlapping key material between the two uses.
const KEY_DERIVATION_DOMAIN: &[u8] = b"captchapi:solution-hash:v1";

/// Hashes and verifies CAPTCHA solutions with a keyed one-way function.
#[derive(Clone)]
pub struct SolutionHasher {
    key: [u8; 32],
}

impl SolutionHasher {
    /// Derive a hashing key from the configured secret.
    ///
    /// Rotating the secret invalidates every in-flight session; because
    /// sessions are short-lived this only affects CAPTCHAs issued before the
    /// restart, which fail validation and expire normally.
    pub fn new(secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DERIVATION_DOMAIN);
        hasher.update(secret.as_bytes());
        let mut key = [0u8; 32];
        key.copy_from_slice(&hasher.finalize());
        Self { key }
    }

    /// Compute the value to persist for a solution, salted with the session ID.
    pub fn hash(&self, session_id: &str, solution: &str) -> String {
        hex::encode(hmac_sha256(
            &self.key,
            &[session_id.as_bytes(), &[0x00], solution.as_bytes()],
        ))
    }

    /// Check a submitted solution against the stored hash in constant time.
    ///
    /// Matching is case-sensitive: the hash is taken over the exact bytes.
    pub fn verify(&self, session_id: &str, solution: &str, stored_hash: &str) -> bool {
        let expected = self.hash(session_id, solution);
        expected.as_bytes().ct_eq(stored_hash.as_bytes()).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-solution-secret-1234";
    const SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";

    // -----------------------------------------------------------------------
    // SolutionHasher
    // -----------------------------------------------------------------------

    #[test]
    fn test_hash_is_deterministic() {
        let hasher = SolutionHasher::new(SECRET);

        let first = hasher.hash(SESSION_ID, "AbC12");
        let second = hasher.hash(SESSION_ID, "AbC12");

        assert_eq!(first, second);
        assert_eq!(first.len(), 64, "HMAC-SHA256 is 64 hex characters");
    }

    #[test]
    fn test_hash_does_not_contain_plaintext() {
        let hasher = SolutionHasher::new(SECRET);

        let hash = hasher.hash(SESSION_ID, "AbC12");

        assert!(!hash.contains("AbC12"));
        assert!(!hash.to_lowercase().contains("abc12"));
    }

    #[test]
    fn test_same_solution_different_sessions_differ() {
        let hasher = SolutionHasher::new(SECRET);

        let first = hasher.hash("session-one", "SAME1");
        let second = hasher.hash("session-two", "SAME1");

        assert_ne!(
            first, second,
            "Session ID salt should prevent identical solutions from colliding"
        );
    }

    #[test]
    fn test_different_solutions_differ() {
        let hasher = SolutionHasher::new(SECRET);

        assert_ne!(
            hasher.hash(SESSION_ID, "AAAA1"),
            hasher.hash(SESSION_ID, "AAAA2")
        );
    }

    #[test]
    fn test_different_secrets_differ() {
        let first = SolutionHasher::new("secret-one-minimum-16chars");
        let second = SolutionHasher::new("secret-two-minimum-16chars");

        assert_ne!(
            first.hash(SESSION_ID, "AbC12"),
            second.hash(SESSION_ID, "AbC12"),
            "A stolen database should be useless without the server secret"
        );
    }

    #[test]
    fn test_salt_and_solution_boundary_is_unambiguous() {
        let hasher = SolutionHasher::new(SECRET);

        // Without a separator, ("ab", "cd") and ("abc", "d") would hash the same.
        assert_ne!(hasher.hash("ab", "cd"), hasher.hash("abc", "d"));
    }

    #[test]
    fn test_verify_accepts_correct_solution() {
        let hasher = SolutionHasher::new(SECRET);
        let stored = hasher.hash(SESSION_ID, "AbC12");

        assert!(hasher.verify(SESSION_ID, "AbC12", &stored));
    }

    #[test]
    fn test_verify_rejects_wrong_solution() {
        let hasher = SolutionHasher::new(SECRET);
        let stored = hasher.hash(SESSION_ID, "AbC12");

        assert!(!hasher.verify(SESSION_ID, "XyZ99", &stored));
    }

    #[test]
    fn test_verify_is_case_sensitive() {
        let hasher = SolutionHasher::new(SECRET);
        let stored = hasher.hash(SESSION_ID, "AbC12");

        assert!(!hasher.verify(SESSION_ID, "abc12", &stored));
    }

    #[test]
    fn test_verify_rejects_correct_solution_for_other_session() {
        let hasher = SolutionHasher::new(SECRET);
        let stored = hasher.hash("session-one", "AbC12");

        assert!(!hasher.verify("session-two", "AbC12", &stored));
    }

    #[test]
    fn test_verify_rejects_empty_and_malformed_stored_hash() {
        let hasher = SolutionHasher::new(SECRET);

        assert!(!hasher.verify(SESSION_ID, "AbC12", ""));
        assert!(!hasher.verify(SESSION_ID, "AbC12", "not-a-hash"));
    }

    #[test]
    fn test_verify_rejects_plaintext_stored_as_hash() {
        // Rows written before this change hold the plaintext answer; they must
        // not validate against the plaintext submission.
        let hasher = SolutionHasher::new(SECRET);

        assert!(!hasher.verify(SESSION_ID, "AbC12", "AbC12"));
    }

    #[test]
    fn test_hash_golden_value() {
        // Regression test: pins the stored format so an accidental change to the
        // key derivation or message layout fails loudly instead of silently
        // invalidating every live session.
        let hasher = SolutionHasher::new("my-test-salt-1234");
        assert_eq!(
            hasher.hash("11111111-2222-3333-4444-555555555555", "AbC12"),
            "dc14bc6a2097b234268f0be305b7281a6be374828fd733f6e2bbebf07893fa0e"
        );
    }
}
