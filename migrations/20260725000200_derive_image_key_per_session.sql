-- Derive the image encryption key per session instead of using one key for all.
--
-- The key is now HMAC-SHA256(master_key, info || session_id), which binds a
-- ciphertext to its row through the key itself and guarantees each key encrypts
-- exactly one message. Rows written under the previous scheme carry version byte
-- 1 and cannot be decrypted with the new keys; sessions are short-lived
-- (max TTL 1 hour), so they are dropped rather than migrated.
DELETE FROM sessions;
