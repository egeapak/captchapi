# Architecture

## Technology Stack

| Component | Technology | Version |
|-----------|-----------|---------|
| Language | Rust | Edition 2021 |
| Web Framework | Axum | 0.8 |
| Async Runtime | Tokio | 1.x |
| Database | SQLite via SQLx | 0.8 |
| CAPTCHA Generation | [captcha-rs](https://github.com/samirdjelal/captcha-rs) | 0.5 |
| Authentication | SHA256 + salt | - |
| Telemetry | OpenTelemetry + tracing | 0.27 |
| Rate Limiting | tower_governor (GCRA) | 0.8 |
| Deployment | Static musl binary in distroless container | - |

## Project Structure

```
src/
├── main.rs                  # Entry point (thin wrapper)
├── lib.rs                   # Library crate exports
├── app.rs                   # App builder (router, services, middleware)
├── config.rs                # Environment configuration
├── error.rs                 # Error types and handling
├── metrics.rs               # Prometheus-style metrics
├── telemetry.rs             # OpenTelemetry tracing setup
├── validation.rs            # Centralized input validation
├── models/                  # Data structures
│   ├── mod.rs
│   ├── session.rs           # Session models
│   ├── session_config.rs    # Session configuration
│   └── api_key.rs           # API key models
├── services/                # Business logic layer
│   ├── mod.rs
│   ├── captcha.rs           # CAPTCHA generation
│   ├── auth.rs              # API key hashing
│   ├── storage.rs           # Database operations
│   ├── session_ops.rs       # Session orchestration
│   ├── api_key_ops.rs       # API key orchestration
│   └── rate_limiter.rs      # Rate limiter configuration
├── routes/                  # HTTP endpoints
│   ├── mod.rs
│   ├── health.rs            # Health check
│   ├── sessions.rs          # Session CRUD
│   ├── api_keys.rs          # API key management
│   └── admin.rs             # Admin operations
├── middleware/              # HTTP middleware
│   ├── mod.rs
│   ├── auth.rs              # Authentication
│   ├── metrics.rs           # Request duration tracking
│   └── request_id.rs        # Request ID injection
└── tasks/                   # Background tasks
    ├── mod.rs
    └── cleanup.rs           # Expired session cleanup
```

## Layered Architecture

```
HTTP Request
    │
    ▼
┌──────────────────────┐
│  Middleware Layer     │  auth, metrics, request_id, rate limiting
├──────────────────────┤
│  Routes Layer        │  HTTP handlers (routes/)
├──────────────────────┤
│  Service Layer       │  Business logic (services/*_ops.rs)
├──────────────────────┤
│  Storage Layer       │  Database operations (services/storage.rs)
├──────────────────────┤
│  Database            │  SQLite via SQLx
└──────────────────────┘
```

### Request Flow

1. **Rate Limiter** (tower_governor) checks per-IP request rate
2. **Middleware** injects request ID, records metrics, validates API key
3. **Route handler** deserializes request and calls the service layer
4. **Service layer** orchestrates business logic (validation, CAPTCHA generation, session management)
5. **Storage layer** executes SQL queries against SQLite

### Key Design Decisions

- **`app.rs` builder pattern**: All router assembly, service instantiation, and middleware wiring lives in `build_app()`, making it testable and reusable across main and integration tests.
- **Centralized validation**: All input validation lives in `validation.rs`, shared between the HTTP server and NAPI bindings.
- **In-process SQLite**: Zero network overhead, no external database dependency. The database file is created automatically on startup.
- **Background cleanup**: A Tokio task runs periodically to delete expired sessions without blocking request handling.

## Database Schema

### Sessions Table

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution_hash TEXT NOT NULL,      -- HMAC-SHA256 of the answer (see Security Model)
    image_bytes BLOB NOT NULL,        -- Raw JPEG image bytes
    created_at INTEGER NOT NULL,      -- Unix timestamp
    expires_at INTEGER NOT NULL,      -- Unix timestamp
    attempt_count INTEGER DEFAULT 0,  -- Failed attempts
    difficulty INTEGER DEFAULT 5,     -- 1-10
    width INTEGER DEFAULT 220,        -- Pixels
    height INTEGER DEFAULT 120,       -- Pixels
    dark_mode INTEGER DEFAULT 0       -- Boolean
);
```

### API Keys Table

```sql
CREATE TABLE api_keys (
    key_hash TEXT PRIMARY KEY,        -- SHA256(key + salt)
    description TEXT,                 -- Human-readable label
    created_at INTEGER NOT NULL,      -- Unix timestamp
    last_used_at INTEGER,             -- Unix timestamp
    is_active INTEGER DEFAULT 1       -- Boolean
);
```

## Security Model

### Authentication

- **Regular API keys**: Required for session CRUD operations. Keys are hashed with SHA256 + configurable salt before storage. Plaintext key is only returned once at creation time.
- **Master API key**: Required for admin operations (API key management, manual cleanup). Set via environment variable.
- **Public endpoints**: Health check, session details retrieval, and CAPTCHA image serving require no authentication.

### Session Security

- Sessions auto-expire based on configurable TTL
- Maximum 3 validation attempts per session (configurable)
- Sessions are deleted after successful validation
- Case-sensitive solution matching, compared in constant time
- CAPTCHA solution is never returned in API responses
- Solutions are stored as `HMAC-SHA256(server_secret, session_id || solution)`, never in plaintext.
  The key is derived from `SOLUTION_HASH_SECRET` (falling back to `API_KEY_SALT`) and never lives
  in the database, so a leaked database file does not reveal answers — a bare digest would not
  help, since a 5-character alphanumeric keyspace is brute-forced in milliseconds. The session ID
  acts as a per-session salt so identical answers do not produce identical hashes.
  Note that `image_bytes` still holds the rendered challenge, which can be OCR'd; hashing removes
  the trivial `SELECT solution FROM sessions` path, it does not make a database leak harmless.

### Rate Limiting

Per-IP rate limiting using the GCRA algorithm via tower_governor. Supports reverse proxy mode (`X-Forwarded-For`, `X-Real-IP`, `Forwarded` headers) for deployment behind nginx/Cloudflare.
