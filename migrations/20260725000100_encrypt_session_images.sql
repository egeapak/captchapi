-- Store the CAPTCHA image encrypted at rest instead of as a raw JPEG.
--
-- The rendered image is the answer in visual form, so leaving it in plaintext
-- would undo the point of hashing the solution. Existing rows hold raw JPEGs
-- that the new read path cannot decrypt, and sessions are short-lived
-- (max TTL 1 hour), so they are dropped rather than migrated.
DELETE FROM sessions;

ALTER TABLE sessions RENAME COLUMN image_bytes TO image_encrypted;
