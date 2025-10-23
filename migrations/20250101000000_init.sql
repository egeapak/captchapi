-- Create sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution TEXT NOT NULL,           -- Correct answer (stored lowercase)
    image_base64 TEXT NOT NULL,       -- Base64 encoded PNG
    created_at INTEGER NOT NULL,      -- Unix timestamp (seconds)
    expires_at INTEGER NOT NULL,      -- Unix timestamp (seconds)
    attempt_count INTEGER DEFAULT 0,  -- Failed validation attempts
    difficulty INTEGER DEFAULT 5,     -- Complexity level (1-10)
    width INTEGER DEFAULT 220,        -- Image width in pixels
    height INTEGER DEFAULT 120,       -- Image height in pixels
    dark_mode INTEGER DEFAULT 0       -- Boolean: 0=light, 1=dark
);

-- Create indexes for sessions table
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_created_at ON sessions(created_at);

-- Create API keys table
CREATE TABLE IF NOT EXISTS api_keys (
    key_hash TEXT PRIMARY KEY,        -- SHA256(api_key)
    description TEXT,                 -- Human-readable label
    created_at INTEGER NOT NULL,      -- Unix timestamp
    last_used_at INTEGER,             -- Unix timestamp
    is_active INTEGER DEFAULT 1       -- Boolean: 0=disabled, 1=active
);

-- Create index for API keys table
CREATE INDEX IF NOT EXISTS idx_api_keys_active ON api_keys(is_active);
