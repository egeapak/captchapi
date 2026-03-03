use sha2::{Digest, Sha256};

pub struct AuthService {
    salt: String,
}

impl AuthService {
    pub fn new(salt: String) -> Self {
        Self { salt }
    }

    pub fn hash_api_key(&self, api_key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        hasher.update(self.salt.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_api_key_produces_consistent_hash() {
        let auth_service = AuthService::new("test-salt".to_string());
        let api_key = "test-key-123";

        let hash1 = auth_service.hash_api_key(api_key);
        let hash2 = auth_service.hash_api_key(api_key);

        assert_eq!(hash1, hash2, "Same key should produce same hash");
        assert_eq!(hash1.len(), 64, "SHA256 hash should be 64 hex chars");
    }

    #[test]
    fn test_hash_api_key_different_keys_different_hashes() {
        let auth_service = AuthService::new("test-salt".to_string());

        let hash1 = auth_service.hash_api_key("key1");
        let hash2 = auth_service.hash_api_key("key2");

        assert_ne!(
            hash1, hash2,
            "Different keys should produce different hashes"
        );
    }

    #[test]
    fn test_hash_api_key_different_salts_different_hashes() {
        let service1 = AuthService::new("salt1".to_string());
        let service2 = AuthService::new("salt2".to_string());

        let hash1 = service1.hash_api_key("same-key");
        let hash2 = service2.hash_api_key("same-key");

        assert_ne!(
            hash1, hash2,
            "Different salts should produce different hashes"
        );
    }

    #[test]
    fn test_hash_api_key_empty_key() {
        let auth_service = AuthService::new("test-salt".to_string());
        let hash = auth_service.hash_api_key("");

        assert_eq!(hash.len(), 64, "Empty key should still produce valid hash");
    }

    #[test]
    fn test_hash_api_key_golden_value() {
        // Regression test: SHA256("test-key-123" + "my-test-salt-1234")
        // This golden value prevents silent changes to hashing behavior.
        let auth_service = AuthService::new("my-test-salt-1234".to_string());
        let hash = auth_service.hash_api_key("test-key-123");
        assert_eq!(
            hash, "f2f4d31d7675db7bd4b3836fe8eff60666984159a71f4643c22e9d5111610600",
            "Hash must match golden value for 'test-key-123' with salt 'my-test-salt-1234'"
        );
    }
}
