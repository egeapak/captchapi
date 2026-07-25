-- Store a keyed hash of the CAPTCHA solution instead of the plaintext answer.
--
-- Existing rows hold plaintext solutions that cannot be verified against the new
-- column, and sessions are short-lived (max TTL 1 hour), so they are dropped
-- rather than migrated. Clients holding a session ID simply request a new CAPTCHA.
DELETE FROM sessions;

ALTER TABLE sessions RENAME COLUMN solution TO solution_hash;
