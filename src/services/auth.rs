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
