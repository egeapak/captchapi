//! Authenticated encryption of stored CAPTCHA images.
//!
//! The rendered challenge is the answer in visual form: anyone who can read
//! `sessions.image_encrypted` and run OCR solves the CAPTCHA, which would make
//! hashing the solution pointless on its own. Images are therefore encrypted
//! with ChaCha20-Poly1305 under a server-side key that never reaches the
//! database, so a leaked database file yields neither the answer nor the image.
//!
//! The session ID is authenticated as associated data, which binds a ciphertext
//! to its row: moving a blob from one session to another, or tampering with the
//! bytes, fails decryption instead of silently serving the wrong image.
//!
//! Stored layout: `[version: 1][nonce: 12][ciphertext || Poly1305 tag: 16]`.

use crate::error::{AppError, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::Rng;
use sha2::{Digest, Sha256};

/// Domain separation for the derived encryption key, so that the same secret
/// used elsewhere (e.g. `API_KEY_SALT`) never produces overlapping key material.
const KEY_DERIVATION_DOMAIN: &[u8] = b"captchapi:image-encryption:v1";

/// Format version of the stored blob. Bump when the layout or cipher changes so
/// old rows can be recognised rather than misparsed.
const FORMAT_VERSION: u8 = 1;

/// ChaCha20-Poly1305 nonce length in bytes.
const NONCE_LEN: usize = 12;

/// Poly1305 authentication tag length in bytes.
const TAG_LEN: usize = 16;

/// Encrypts and decrypts CAPTCHA images for storage at rest.
#[derive(Clone)]
pub struct ImageCipher {
    cipher: ChaCha20Poly1305,
}

impl ImageCipher {
    /// Derive an encryption key from the configured secret.
    ///
    /// Rotating the secret makes existing rows undecryptable; because sessions
    /// are short-lived, only CAPTCHAs issued before the restart are affected —
    /// their image requests fail and they expire normally.
    pub fn new(secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DERIVATION_DOMAIN);
        hasher.update(secret.as_bytes());
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&hasher.finalize());

        Self {
            cipher: ChaCha20Poly1305::new(&key_bytes.into()),
        }
    }

    /// Encrypt an image for storage, binding it to `session_id`.
    pub fn encrypt(&self, session_id: &str, image_bytes: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        let ciphertext = self
            .cipher
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

    /// Decrypt a stored image, verifying it belongs to `session_id`.
    ///
    /// Fails for tampered bytes, a mismatched session ID, a wrong key, or a
    /// layout this build does not understand.
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

        self.cipher
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

        // The session ID is authenticated, so a blob cannot be moved between rows
        assert!(cipher.decrypt("session-two", &stored).is_err());
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
