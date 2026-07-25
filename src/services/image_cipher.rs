//! Authenticated encryption of stored CAPTCHA images.
//!
//! The rendered challenge is the answer in visual form: anyone who can read
//! `sessions.image_encrypted` and run OCR solves the CAPTCHA, which would make
//! hashing the solution pointless on its own. Images are therefore encrypted
//! with ChaCha20-Poly1305 under a server-side key that never reaches the
//! database, so a leaked database file yields neither the answer nor the image.
//!
//! Every session gets its own key, derived as
//! `HMAC-SHA256(master_key, "…image-key:v1" || 0x00 || session_id)`. That binds a
//! ciphertext to its row structurally: a blob moved to another session cannot be
//! decrypted, because the key for that row is a different key. It also means each
//! key ever encrypts exactly one message, so nonce reuse — the one way to break
//! ChaCha20-Poly1305 with correct code — cannot happen even in principle.
//!
//! The session ID is *also* authenticated as associated data. That is redundant
//! given the derived key, and deliberately so: it keeps the binding intact if the
//! key derivation is ever simplified back to a single key.
//!
//! Stored layout, in bytes:
//! `[scheme version: 1][nonce: 12][ciphertext || Poly1305 tag: 16]`.

use crate::error::{AppError, Result};
use crate::services::hmac::hmac_sha256;
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::Rng;
use sha2::{Digest, Sha256};

/// Domain separation for the master key, so that the same secret used elsewhere
/// (e.g. `API_KEY_SALT`) never produces overlapping key material.
const KEY_DERIVATION_DOMAIN: &[u8] = b"captchapi:image-encryption:v1";

/// Domain separation for per-session keys derived from the master key.
const SESSION_KEY_INFO: &[u8] = b"captchapi:image-key:v1";

/// Version of the encryption scheme — key derivation and stored layout together.
/// Bump when either changes, so rows written by an older build fail with a clear
/// error instead of a generic authentication failure.
const FORMAT_VERSION: u8 = 2;

/// ChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 12;

/// Poly1305 authentication tag length in bytes.
const TAG_LEN: usize = 16;

/// Encrypts and decrypts CAPTCHA images for storage at rest.
#[derive(Clone)]
pub struct ImageCipher {
    master_key: [u8; 32],
}

impl ImageCipher {
    /// Derive the master key from the configured secret.
    ///
    /// Rotating the secret makes existing rows undecryptable; because sessions
    /// are short-lived, only CAPTCHAs issued before the restart are affected —
    /// their image requests fail and they expire normally.
    pub fn new(secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DERIVATION_DOMAIN);
        hasher.update(secret.as_bytes());
        let mut master_key = [0u8; 32];
        master_key.copy_from_slice(&hasher.finalize());

        Self { master_key }
    }

    /// Build the cipher for one session's key.
    ///
    /// Costs a single HMAC per call — negligible next to encoding or encrypting
    /// the image itself, and it keeps the key out of any long-lived state.
    fn session_cipher(&self, session_id: &str) -> ChaCha20Poly1305 {
        let key = hmac_sha256(
            &self.master_key,
            &[SESSION_KEY_INFO, &[0x00], session_id.as_bytes()],
        );
        ChaCha20Poly1305::new(&key.into())
    }

    /// Encrypt an image for storage under `session_id`'s own key.
    pub fn encrypt(&self, session_id: &str, image_bytes: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        let ciphertext = self
            .session_cipher(session_id)
            .encrypt(
                &nonce,
                Payload {
                    msg: image_bytes,
                    aad: session_id.as_bytes(),
                },
            )
            .map_err(|_| {
                // The AEAD error type is deliberately opaque and carries no detail.
                AppError::Internal(anyhow::anyhow!("Failed to encrypt CAPTCHA image"))
            })?;

        let mut stored = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        stored.push(FORMAT_VERSION);
        stored.extend_from_slice(&nonce_bytes);
        stored.extend_from_slice(&ciphertext);
        Ok(stored)
    }

    /// Decrypt a stored image with `session_id`'s key.
    ///
    /// Fails for tampered bytes, a mismatched session ID, a wrong master secret,
    /// or a scheme version this build does not understand.
    pub fn decrypt(&self, session_id: &str, stored: &[u8]) -> Result<Vec<u8>> {
        if stored.len() < 1 + NONCE_LEN + TAG_LEN {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Stored CAPTCHA image is too short to be valid"
            )));
        }
        if stored[0] != FORMAT_VERSION {
            return Err(AppError::Internal(anyhow::anyhow!(
                "Unsupported stored CAPTCHA image format version: {}",
                stored[0]
            )));
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&stored[1..1 + NONCE_LEN]);
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext = &stored[1 + NONCE_LEN..];

        self.session_cipher(session_id)
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: session_id.as_bytes(),
                },
            )
            .map_err(|_| {
                AppError::Internal(anyhow::anyhow!(
                    "Failed to decrypt CAPTCHA image (wrong key or tampered data)"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-image-secret-1234";
    const SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";

    /// Recover the raw per-session key for assertions about the derivation.
    /// Mirrors `session_cipher`, which returns an opaque cipher instance.
    fn derived_key(cipher: &ImageCipher, session_id: &str) -> [u8; 32] {
        hmac_sha256(
            &cipher.master_key,
            &[SESSION_KEY_INFO, &[0x00], session_id.as_bytes()],
        )
    }

    /// A stand-in for a rendered JPEG: starts with the JPEG signature so tests
    /// can assert the marker is not visible in the stored blob.
    fn sample_image() -> Vec<u8> {
        let mut image = vec![0xFF, 0xD8, 0xFF, 0xE0];
        image.extend((0u8..=255).cycle().take(2048));
        image
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let cipher = ImageCipher::new(SECRET);
        let image = sample_image();

        let stored = cipher.encrypt(SESSION_ID, &image).unwrap();
        let recovered = cipher.decrypt(SESSION_ID, &stored).unwrap();

        assert_eq!(recovered, image);
    }

    #[test]
    fn test_roundtrip_of_empty_input() {
        let cipher = ImageCipher::new(SECRET);

        let stored = cipher.encrypt(SESSION_ID, &[]).unwrap();
        let recovered = cipher.decrypt(SESSION_ID, &stored).unwrap();

        assert!(recovered.is_empty());
    }

    #[test]
    fn test_stored_blob_does_not_expose_plaintext() {
        let cipher = ImageCipher::new(SECRET);
        let image = sample_image();

        let stored = cipher.encrypt(SESSION_ID, &image).unwrap();

        assert_ne!(stored, image);
        assert_ne!(
            &stored[1 + NONCE_LEN..1 + NONCE_LEN + 3],
            &[0xFF, 0xD8, 0xFF],
            "The JPEG signature must not survive into the stored blob"
        );
        assert!(
            !stored
                .windows(image.len().min(64))
                .any(|w| w == &image[..image.len().min(64)]),
            "No plaintext run should appear in the stored blob"
        );
    }

    #[test]
    fn test_stored_layout_is_version_nonce_ciphertext() {
        let cipher = ImageCipher::new(SECRET);
        let image = sample_image();

        let stored = cipher.encrypt(SESSION_ID, &image).unwrap();

        assert_eq!(stored[0], FORMAT_VERSION);
        assert_eq!(
            stored.len(),
            1 + NONCE_LEN + image.len() + TAG_LEN,
            "Overhead should be version byte + nonce + Poly1305 tag"
        );
    }

    #[test]
    fn test_each_encryption_uses_a_fresh_nonce() {
        let cipher = ImageCipher::new(SECRET);
        let image = sample_image();

        let first = cipher.encrypt(SESSION_ID, &image).unwrap();
        let second = cipher.encrypt(SESSION_ID, &image).unwrap();

        assert_ne!(
            &first[1..1 + NONCE_LEN],
            &second[1..1 + NONCE_LEN],
            "Nonces must not repeat for the same key"
        );
        assert_ne!(
            first, second,
            "Encrypting the same image twice must not produce the same blob"
        );
    }

    #[test]
    fn test_decrypt_rejects_wrong_session_id() {
        let cipher = ImageCipher::new(SECRET);
        let stored = cipher.encrypt("session-one", &sample_image()).unwrap();

        // Another session's key simply cannot open this blob
        assert!(cipher.decrypt("session-two", &stored).is_err());
    }

    // -----------------------------------------------------------------------
    // Per-session key derivation
    // -----------------------------------------------------------------------

    #[test]
    fn test_session_keys_differ_between_sessions() {
        let cipher = ImageCipher::new(SECRET);

        let first = derived_key(&cipher, "session-one");
        let second = derived_key(&cipher, "session-two");

        assert_ne!(
            first, second,
            "Each session must get its own encryption key"
        );
    }

    #[test]
    fn test_session_key_is_stable_for_the_same_session() {
        let cipher = ImageCipher::new(SECRET);

        assert_eq!(
            derived_key(&cipher, SESSION_ID),
            derived_key(&cipher, SESSION_ID),
            "Derivation must be deterministic, or stored images become unreadable"
        );
    }

    #[test]
    fn test_session_key_differs_from_the_master_key() {
        let cipher = ImageCipher::new(SECRET);

        assert_ne!(
            derived_key(&cipher, SESSION_ID),
            cipher.master_key,
            "The master key must never be used directly to encrypt"
        );
    }

    #[test]
    fn test_session_keys_differ_between_master_secrets() {
        let first = ImageCipher::new(SECRET);
        let second = ImageCipher::new("a-completely-different-secret");

        assert_ne!(
            derived_key(&first, SESSION_ID),
            derived_key(&second, SESSION_ID)
        );
    }

    #[test]
    fn test_session_key_derivation_is_domain_separated() {
        // The image key must not collide with the solution hash of the same
        // session under the same secret, even though both are HMAC-SHA256.
        let cipher = ImageCipher::new(SECRET);
        let solution_hasher = crate::services::SolutionHasher::new(SECRET);

        assert_ne!(
            hex::encode(derived_key(&cipher, SESSION_ID)),
            solution_hasher.hash(SESSION_ID, ""),
        );
    }

    #[test]
    fn test_session_id_boundary_is_unambiguous() {
        // The separator keeps the info label from running into the session ID,
        // so no crafted session ID can collide with another session's key.
        let cipher = ImageCipher::new(SECRET);

        assert_ne!(derived_key(&cipher, ""), derived_key(&cipher, "\u{0}"));
    }

    #[test]
    fn test_every_session_key_encrypts_at_most_one_message_in_practice() {
        // Two sessions encrypting the same image must share nothing: different
        // keys and different nonces.
        let cipher = ImageCipher::new(SECRET);
        let image = sample_image();

        let first = cipher.encrypt("session-one", &image).unwrap();
        let second = cipher.encrypt("session-two", &image).unwrap();

        assert_ne!(&first[1..1 + NONCE_LEN], &second[1..1 + NONCE_LEN]);
        assert_ne!(
            &first[1 + NONCE_LEN..],
            &second[1 + NONCE_LEN..],
            "Same plaintext under different session keys must not collide"
        );
    }

    #[test]
    fn test_decrypt_rejects_wrong_key() {
        let writer = ImageCipher::new(SECRET);
        let reader = ImageCipher::new("a-completely-different-secret");
        let stored = writer.encrypt(SESSION_ID, &sample_image()).unwrap();

        assert!(
            reader.decrypt(SESSION_ID, &stored).is_err(),
            "A stolen database should be useless without the server secret"
        );
    }

    #[test]
    fn test_decrypt_rejects_tampered_ciphertext() {
        let cipher = ImageCipher::new(SECRET);
        let mut stored = cipher.encrypt(SESSION_ID, &sample_image()).unwrap();

        // Flip a bit in the ciphertext body
        let mid = stored.len() / 2;
        stored[mid] ^= 0b0000_0001;

        assert!(cipher.decrypt(SESSION_ID, &stored).is_err());
    }

    #[test]
    fn test_decrypt_rejects_tampered_nonce() {
        let cipher = ImageCipher::new(SECRET);
        let mut stored = cipher.encrypt(SESSION_ID, &sample_image()).unwrap();

        stored[1] ^= 0b0000_0001;

        assert!(cipher.decrypt(SESSION_ID, &stored).is_err());
    }

    #[test]
    fn test_decrypt_rejects_truncated_blob() {
        let cipher = ImageCipher::new(SECRET);
        let stored = cipher.encrypt(SESSION_ID, &sample_image()).unwrap();

        assert!(cipher.decrypt(SESSION_ID, &stored[..8]).is_err());
        assert!(cipher.decrypt(SESSION_ID, &[]).is_err());
    }

    #[test]
    fn test_decrypt_rejects_unknown_format_version() {
        let cipher = ImageCipher::new(SECRET);
        let mut stored = cipher.encrypt(SESSION_ID, &sample_image()).unwrap();

        stored[0] = FORMAT_VERSION + 1;

        assert!(cipher.decrypt(SESSION_ID, &stored).is_err());
    }

    #[test]
    fn test_decrypt_rejects_previous_scheme_version() {
        // Rows written before per-session key derivation carry version 1 and
        // must fail loudly rather than as an opaque authentication error.
        let cipher = ImageCipher::new(SECRET);
        let mut stored = cipher.encrypt(SESSION_ID, &sample_image()).unwrap();

        stored[0] = 1;

        let err = cipher.decrypt(SESSION_ID, &stored).unwrap_err();
        assert!(
            format!("{err:?}").contains("version"),
            "Old rows should fail on the version check, not the auth tag: {err:?}"
        );
    }

    #[test]
    fn test_decrypt_rejects_plaintext_jpeg() {
        // Rows written before this change hold a raw JPEG; they must be rejected
        // rather than served as-is.
        let cipher = ImageCipher::new(SECRET);

        assert!(cipher.decrypt(SESSION_ID, &sample_image()).is_err());
    }

    #[test]
    fn test_same_secret_produces_interoperable_ciphers() {
        // Two instances built from the same secret must be able to read each
        // other's output (e.g. after a restart).
        let writer = ImageCipher::new(SECRET);
        let reader = ImageCipher::new(SECRET);
        let image = sample_image();

        let stored = writer.encrypt(SESSION_ID, &image).unwrap();

        assert_eq!(reader.decrypt(SESSION_ID, &stored).unwrap(), image);
    }
}
