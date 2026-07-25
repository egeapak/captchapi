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
├── main.rs                  # Entry point (thin wrapper over the library crate)
├── lib.rs                   # Library crate exports
├── app.rs                   # App builder (router, services, middleware)
├── cli.rs                   # Command-line surface, `config show`/`check`, reload client
├── config/                  # Layered, reloadable configuration
│   ├── mod.rs               # Config struct, resolution, redacting Debug
│   ├── params.rs            # PARAMS table: one row per setting, drives everything
│   ├── sources.rs           # CLI / env / env-file / TOML layers, provenance
│   └── handle.rs            # ConfigHandle: watch channel, reload, runtime overrides
├── error.rs                 # Error types and handling
├── metrics.rs               # Prometheus-style metrics
├── telemetry.rs             # OpenTelemetry tracing setup
├── validation.rs            # Centralized input validation
├── models/                  # Data structures
│   ├── mod.rs
│   ├── session.rs           # Session models
│   ├── session_config.rs    # The reloadable subset, snapshotted per request
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
- **Background cleanup**: A Tokio task runs periodically to delete expired sessions without blocking request handling. It watches the config handle, so a reload changes its interval without a restart.

## Configuration

Settings resolve through four layers, highest precedence first: the command line, the process
environment, an env file, then a TOML config file, falling back to built-in defaults.

The key design decision is that **the command line is not a second configuration system**.
Every layer resolves the same canonical keys — the environment variable names — so the CLI
plugs in as just another `EnvProvider`, the trait that already existed for testing. All parsing,
validation and error messages continue to come from `Config::from_env_provider`, in one place.

`PARAMS` (`config/params.rs`) is the single source of truth: one row per setting, carrying its
environment key, flag, TOML path, type, default, help text and whether it is reloadable. Help
output, TOML validation, provenance reporting and the reload partition are all derived from it,
so adding a setting is one row plus one struct field.

### Reload

```
SIGHUP ─┐
CLI ────┼─→ ConfigHandle::reload() ─→ re-resolve ─→ watch::Sender ─┬─→ request handlers
API ────┘                                                          └─→ cleanup task
```

The running `Config` lives behind a `tokio::sync::watch` channel. Readers take a cheap snapshot;
the cleanup task gets change notification, which it needs to adopt a new interval. Resolution
and publication happen under a single lock, so a SIGHUP and an admin request cannot interleave
into a lost update.

Only values read per request or per tick can change: session TTLs, the attempt limit, JPEG
compression and the cleanup interval — exactly the fields of `SessionConfig`. Everything else is
captured at startup into the listener, the connection pool, the middleware or the rate limiter,
and a reload reports drift on those rather than pretending to apply it. A reload that fails to
resolve is logged and discarded; the running server is never taken down by a bad config edit.

Handlers take **one** snapshot per request, so a reload landing mid-request cannot make a traced
value disagree with the value used for validation.

### Secret handling

Secrets are accepted as file paths on the command line, never as flag values, keeping them out
of `ps`, shell history and `docker inspect`. They have no TOML key at all, so a config file is
safe to commit. `Config` and `Cli` carry hand-written `Debug` impls that redact them, and
`config show` and the admin API redact them too — the admin endpoint exists to explain the
server's behaviour, not to read credentials back out of it.

## Database Schema

### Sessions Table

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution TEXT NOT NULL,           -- Correct answer
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
- Case-sensitive solution matching
- CAPTCHA solution is never returned in API responses

### Rate Limiting

Per-IP rate limiting using the GCRA algorithm via tower_governor. Supports reverse proxy mode (`X-Forwarded-For`, `X-Real-IP`, `Forwarded` headers) for deployment behind nginx/Cloudflare.
