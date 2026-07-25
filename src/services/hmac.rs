//! HMAC-SHA256 (RFC 2104), shared by the solution hasher and the image cipher.
//!
//! Kept as one implementation with one set of test vectors so the two callers
//! cannot drift apart.

use sha2::{Digest, Sha256};

/// SHA-256 block size, per RFC 2104.
const BLOCK_SIZE: usize = 64;

/// HMAC-SHA256 over the concatenation of `parts`.
pub(crate) fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut padded_key = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let digest = Sha256::digest(key);
        padded_key[..digest.len()].copy_from_slice(&digest);
    } else {
        padded_key[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36u8; BLOCK_SIZE];
    let mut outer_pad = [0x5cu8; BLOCK_SIZE];
    for (i, byte) in padded_key.iter().enumerate() {
        inner_pad[i] ^= byte;
        outer_pad[i] ^= byte;
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    for part in parts {
        inner.update(part);
    }

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner.finalize());

    let mut result = [0u8; 32];
    result.copy_from_slice(&outer.finalize());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // RFC 4231 test vectors
    // -----------------------------------------------------------------------

    #[test]
    fn test_hmac_sha256_rfc4231_case_1() {
        // Key = 20 bytes of 0x0b, data = "Hi There"
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, &[b"Hi There"]);
        assert_eq!(
            hex::encode(mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_hmac_sha256_rfc4231_case_2() {
        // Key = "Jefe", data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", &[b"what do ya want for nothing?"]);
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn test_hmac_sha256_rfc4231_case_6_key_longer_than_block() {
        // Key = 131 bytes of 0xaa (longer than the 64-byte block, so it is hashed first)
        let key = [0xaau8; 131];
        let mac = hmac_sha256(
            &key,
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"],
        );
        assert_eq!(
            hex::encode(mac),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn test_hmac_sha256_parts_are_concatenated() {
        let key = b"key";
        let split = hmac_sha256(key, &[b"abc", b"def"]);
        let whole = hmac_sha256(key, &[b"abcdef"]);
        assert_eq!(
            split, whole,
            "Message parts should be a plain concatenation"
        );
    }

    #[test]
    fn test_hmac_sha256_distinct_keys_give_distinct_macs() {
        assert_ne!(
            hmac_sha256(b"key-one", &[b"msg"]),
            hmac_sha256(b"key-two", &[b"msg"])
        );
    }
}
