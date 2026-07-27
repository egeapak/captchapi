-- Persisted configuration.
--
-- Values are raw strings in the same canonical form every other configuration layer produces,
-- so parsing and validation stay in Config::from_env_provider and nowhere else. Keyed by the
-- Config field name, which is what the admin API and the console already speak.
CREATE TABLE config_settings (
    field      TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    -- 'admin-api' or 'cli'. Recorded because this table can change how the service listens
    -- and how it rate-limits, so a change to it is worth being able to attribute.
    updated_by TEXT NOT NULL
);

-- One row per write to config_settings, so a configuration that prevents startup rolls back
-- automatically instead of wedging the service.
--
-- The lifecycle is: a write inserts 'pending'; a boot that reads it increments `attempts`; a
-- process that survives long enough marks it 'confirmed'. A boot that finds a 'pending' row
-- which has already been attempted knows the previous try did not survive, and restores the
-- newest 'confirmed' snapshot instead.
CREATE TABLE config_generations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    -- JSON object of field -> value: config_settings as it stood when this row was written.
    snapshot   TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('pending', 'confirmed', 'rolled_back')),
    attempts   INTEGER NOT NULL DEFAULT 0
);

-- Boot reads the newest row, and rollback reads the newest confirmed one.
CREATE INDEX idx_config_generations_status ON config_generations (status, id DESC);
