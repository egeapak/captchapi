# CaptchAPI - Project Documentation

## Overview

CaptchAPI is a REST API service for creating, validating, and consuming CAPTCHA challenges. Built with Rust, it provides a secure, high-performance solution for integrating CAPTCHA verification into applications.

## Documentation

This file provides project overview, architecture, and development workflow. For detailed information:

- **API Documentation**: See `docs/API.md` for complete endpoint specifications, request/response formats, and usage examples

## Architecture

### Technology Stack

- **Language**: Rust (Edition 2021)
- **Web Framework**: Axum 0.8 (async-first, built on Tokio)
- **Database**: SQLite via SQLx 0.9 (async, compile-time checked queries)
- **CAPTCHA Generation**: In-tree renderer (`src/services/captcha/generator.rs`) on image + imageproc, JPEG/text features only
- **Authentication**: API key-based with SHA256 hashing
- **Deployment**: Static musl binary in distroless container (8.49 MB unpacked / 3.36 MB compressed)

### Key Features

- ✅ Configurable CAPTCHA generation (difficulty 1-10, dark mode, custom dimensions, custom length)
- ✅ API key authentication for protected operations
- ✅ Public image retrieval (requires session ID)
- ✅ Automatic session expiration and cleanup
- ✅ Validation attempt limiting (max 3 attempts)
- ✅ Case-sensitive solution matching (constant-time, against a stored keyed hash)
- ✅ CAPTCHA solutions stored as HMAC-SHA256, never in plaintext
- ✅ CAPTCHA images encrypted at rest with ChaCha20-Poly1305
- ✅ CAPTCHA solution not returned in API response (removed in v1.0.0 for security)
- ✅ Structured logging with tracing
- ✅ In-process SQLite database (zero external dependencies)
- ✅ Ultra-small Docker image (full LTO and static linking)
- ✅ Zero runtime dependencies (fully static binary)

## Project Structure

```
captchapi/
├── CLAUDE.md                    # This file - project documentation
├── .env.example                 # Example environment configuration
├── Cargo.toml                   # Rust dependencies
├── Cross.toml                   # Cross-compilation config
├── migrations/                  # SQLx database migrations
│   └── 20250101000000_init.sql
└── src/
    ├── main.rs                  # Application entry point (thin wrapper)
    ├── lib.rs                   # Library crate exports
    ├── app.rs                   # App builder (router, services, middleware)
    ├── config/                  # Layered, reloadable configuration
    │   ├── mod.rs               # Config struct, resolution, redacting Debug
    │   ├── params.rs            # PARAMS: the single source of truth for every setting
    │   ├── sources.rs           # CLI / env / env-file / TOML layers and provenance
    │   └── handle.rs            # ConfigHandle: watch channel, reload, runtime overrides
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
    │   ├── captcha/             # CAPTCHA generation
    │   │   ├── mod.rs           # CaptchaService (JPEG encoding)
    │   │   └── generator.rs     # In-tree renderer (vendored from captcha-rs)
    │   ├── auth.rs              # API key hashing
    │   ├── storage.rs           # Database operations
    │   ├── session_ops.rs       # Session orchestration
    │   ├── solution_hash.rs     # Keyed hashing of CAPTCHA solutions
    │   ├── image_cipher.rs      # Encryption of stored CAPTCHA images
    │   ├── hmac.rs              # Shared HMAC-SHA256 primitive
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

## API Documentation

For complete API documentation including all endpoints, request/response formats, and usage examples, see `docs/API.md`

**Quick Reference:**
- **Public**: Health check, Get session details, Get CAPTCHA image (binary JPEG)
- **Protected**: Create sessions, Validate solutions, Delete sessions
- **Admin**: Manage API keys (create, list, update, delete), manual cleanup, show/patch/reload configuration

The API documentation includes:
- Full endpoint specifications
- Request/response examples
- Authentication requirements
- Error codes and handling
- Complete usage flows
- Migration guides

## Configuration

Configuration can come from the command line, the environment, an env file or a TOML file.
Precedence, highest first:

```
command line > environment > env file (.env) > config file > built-in default
```

**Adding a new setting is two edits:** one row in `PARAMS` (`src/config/params.rs`) and one
field on `Config` (`src/config/mod.rs`). The flag, help text, TOML key, provenance reporting
and the reloadable/boot-only split are all derived from the table. Tests enforce that the two
stay in sync, including that every declared default matches what the code actually produces.

**Reloadable vs boot-only.** Only values read per request or per tick can change at runtime:
session TTLs, the attempt limit, JPEG compression and the cleanup interval. Everything else is
captured at startup by the listener, the connection pool, the middleware or the rate limiter.
A reload reports drift on those rather than pretending to apply it.

Reload is triggered by SIGHUP, `captchapi reload`, or `POST /api/v1/admin/config/reload`.

**Secrets** are file-only on the CLI (`--api-key-salt-file`, `--master-api-key-file`) and cannot
be set in the TOML file at all. `Config` and `Cli` both have hand-written `Debug` impls that
redact them — keep it that way when adding fields.

### CLI

```bash
captchapi                        # run the server (default verb)
captchapi config show            # effective config with provenance, secrets redacted
captchapi config check           # validate and exit (0 ok, 2 bad) — useful in CI
captchapi reload                 # signal a running server to re-read its config
captchapi --help
```

### Environment Variables

Create a `.env` file based on `.env.example`:

```bash
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:./data/captchapi.db
DATABASE_MAX_CONNECTIONS=5

# Security
API_KEY_SALT=CHANGE-THIS-TO-A-RANDOM-SALT-IN-PRODUCTION
MASTER_API_KEY=CHANGE-THIS-TO-A-SECURE-MASTER-KEY-IN-PRODUCTION
# Optional: dedicated key for hashing CAPTCHA solutions (defaults to API_KEY_SALT)
SOLUTION_HASH_SECRET=CHANGE-THIS-TO-A-RANDOM-SECRET-IN-PRODUCTION
# Optional: dedicated key for encrypting stored images (defaults to API_KEY_SALT)
IMAGE_ENCRYPTION_SECRET=CHANGE-THIS-TO-A-RANDOM-SECRET-IN-PRODUCTION

# CAPTCHA Defaults
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3
CAPTCHA_COMPRESSION=40

# Rate Limiting
RATE_LIMIT_REQUESTS_PER_SECOND=2
RATE_LIMIT_BURST_SIZE=10
RATE_LIMIT_REVERSE_PROXY=false    # Set true when behind nginx/Cloudflare

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60

# OpenTelemetry (optional; requires a build with --features otel)
OTEL_ENABLED=false
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=captchapi
```

**Important**:
- Always change `API_KEY_SALT` to a random string in production!
- Always change `MASTER_API_KEY` to a strong, random key in production!
- Rotating `SOLUTION_HASH_SECRET` / `IMAGE_ENCRYPTION_SECRET` (or `API_KEY_SALT`, when no dedicated secret is set) invalidates sessions issued before the restart
- The master key has full administrative access - protect it carefully!
- Only enable `RATE_LIMIT_REVERSE_PROXY` if you trust your proxy — clients can spoof headers otherwise

## Database Schema

### Sessions Table
Stores active CAPTCHA sessions with metadata and solutions.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution_hash TEXT NOT NULL,      -- HMAC-SHA256(secret, id || solution)
    image_encrypted BLOB NOT NULL,    -- ChaCha20-Poly1305 ciphertext of the JPEG
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
Stores hashed API keys for authentication.

```sql
CREATE TABLE api_keys (
    key_hash TEXT PRIMARY KEY,        -- SHA256(key + salt)
    description TEXT,                 -- Human-readable label
    created_at INTEGER NOT NULL,      -- Unix timestamp
    last_used_at INTEGER,             -- Unix timestamp
    is_active INTEGER DEFAULT 1       -- Boolean
);
```

## Development Workflow

### Setup

```bash
# 1. Clone and navigate to project
cd captchapi

# 2. Copy environment template
cp .env.example .env

# 3. Edit .env and set API_KEY_SALT
nano .env

# 4. Build project (downloads dependencies)
cargo build

# 5. Run migrations (creates database)
cargo run  # Migrations run automatically on startup
```

### Code Quality Standards

**IMPORTANT**: After **EVERY** code change, you **MUST** run these steps **IN ORDER**:

#### Step 1: Format Code
```bash
cargo fmt
```

#### Step 2: Run Linter
```bash
cargo clippy
```

#### Step 3: Check Compilation
```bash
cargo check --all-targets --workspace
```

**Use `--workspace`.** `bindings/nodejs` matches on `AppError` exhaustively with no wildcard
arm, so adding a variant breaks that crate — and a bare `cargo check` will not tell you.

#### Step 4: Run Rust Tests
```bash
cargo nextest run
```

#### Step 5: Run API Tests
**Note:** Requires server to be running first.

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run API tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**All five steps must pass with no errors before committing.**

These steps ensure:
- ✅ Consistent code formatting
- ✅ No common mistakes or anti-patterns
- ✅ Code compiles successfully
- ✅ All unit and integration tests pass
- ✅ HTTP API behaves correctly

### Testing

#### Rust Tests

```bash
cargo nextest run                         # Run all tests
cargo nextest run --lib                   # Unit tests only
cargo nextest run --test sessions_test    # Integration tests
```

#### API Tests (Bruno)

**Prerequisites:** Server must be running.

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

#### Complete Test Workflow

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run all tests
cargo nextest run
./.bruno/Tests/Scripts/test-bruno-full.sh
```

## Security Considerations

### API Key Security

1. **Storage**: API keys are hashed with SHA256 + salt before storage
2. **Never** log API keys or include them in error messages
3. **Rotate** the `API_KEY_SALT` if compromised (requires re-creating all keys)

### Session Security

1. **Expiration**: Sessions auto-expire based on TTL
2. **Attempt Limiting**: Max 3 validation attempts per session
3. **Auto-Deletion**: Sessions deleted after successful validation
4. **Cleanup**: Background task removes expired sessions every 60s
5. **Solution Hashing**: Only `HMAC-SHA256(secret, session_id || solution)` is stored. The key comes
   from `SOLUTION_HASH_SECRET` (default: `API_KEY_SALT`) and never lives in the database, so reading
   the database does not reveal answers. A plain digest would be useless here — short alphanumeric
   solutions are brute-forced instantly.
6. **Image Encryption**: Images are stored as ChaCha20-Poly1305 ciphertext and decrypted only when
   served — otherwise the stored challenge could simply be OCR'd. Each session uses its own key,
   `HMAC-SHA256(master_key, info || session_id)`, with the master key from `IMAGE_ENCRYPTION_SECRET`
   (default `API_KEY_SALT`). Per-session keys make cross-row reuse and nonce reuse impossible; the
   session ID is also passed as associated data for defence in depth.
7. **Airgapped Solutions**: No API returns the answer to a stored session — not the HTTP API, not the
   NAPI bindings. Use the stateless `generate()` binding if you need the plaintext without storage.

### Best Practices

- ✅ Use HTTPS in production
- ✅ Set strong, random `API_KEY_SALT`
- ✅ Rate limiting (implemented via reverse proxy support)
- ✅ Monitor for unusual patterns (multiple failed validations)
- ✅ Keep dependencies updated

## Dependencies

### Core Dependencies

```toml
axum = "0.8"                    # Web framework
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync", "signal"] }
sqlx = { version = "0.9", features = ["runtime-tokio", "sqlite", "migrate"] }

# CAPTCHA rendering. Deliberately minimal features: only JPEG is encoded and
# only text/shape drawing is used. `image`'s defaults would add every codec
# (AVIF, EXR, TIFF, PNG, WebP, ...) and ~58 transitive crates.
image = { version = "0.25", default-features = false, features = ["jpeg"] }
imageproc = { version = "0.26", default-features = false, features = ["text"] }
ab_glyph = "0.2"

serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
sha2 = "0.11"                   # Hashing
chacha20poly1305 = { version = "0.11", default-features = false, features = ["alloc", "getrandom"] }
pico-args = { version = "0.5", features = ["eq-separator"] }
toml = { version = "1", default-features = false, features = ["std", "parse", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"                 # Logging
rand = "0.10"                   # Random generation
```

### Cargo Features

| Feature | Default | Effect |
|---------|---------|--------|
| `otel`  | off     | Compiles in the OpenTelemetry OTLP trace exporter |

`otel` is off by default because the OTLP exporter pulls a full HTTP client
(reqwest and friends) into the binary — roughly 700 KB — for a path that is
inert unless `OTEL_ENABLED` is set at runtime.

**Release builds enable it explicitly.** `release.yml` and the `justfile`
both pass `--features otel` to `cross build`, so the published container
images keep the exporter. Only plain `cargo build` omits it.

Note the split: the `opentelemetry` **API** crate is an unconditional
dependency because `src/metrics.rs` builds every counter and histogram on it.
Only the SDK, the OTLP exporter and `tracing-opentelemetry` are gated, so
metrics work in every build.

A binary built without `otel` warns on stderr at startup if `OTEL_ENABLED` is
set, rather than dropping traces silently.

**Both configurations must be linted**, since `cfg(not(feature = "otel"))`
paths are invisible to `--all-features`:

```bash
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

### SQLite Build Flags

`.cargo/config.toml` sets `LIBSQLITE3_FLAGS` to strip the bundled SQLite
amalgamation down to what the service uses — no FTS, R-tree, STAT4, JSON1,
soundex, deprecated shims or extension loading. That is ~332 KB of binary
for six query shapes that touch none of it. `SQLITE_DQS=0` additionally
rejects double-quoted string literals, so a mistyped identifier errors
instead of silently becoming a string.

**Every workspace member must depend on sqlx with `sqlite-bundled`, never
`sqlite`.** The latter enables sqlx's `sqlite-load-extension` feature, whose
bindings reference `sqlite3_load_extension` — a symbol that does not exist in
a library built with `SQLITE_OMIT_LOAD_EXTENSION`. Cargo unifies features
across the workspace, so a single member requesting `sqlite` breaks the
entire build with `undefined symbol: sqlite3_load_extension`.

### Vendored CAPTCHA Renderer

`src/services/captcha/generator.rs` is vendored from `captcha-rs` v0.5.0 (MIT)
rather than used as a dependency, for two reasons:

1. Upstream depends on `imageproc` with default features, whose `default`
   list includes `image/default`. Because Cargo features are additive, that
   re-enables every image codec no matter what this crate declares. Vendoring
   is the only way to hold the feature set down.
2. Upstream embeds Monotype Arial, whose license forbids redistribution. The
   renderer uses Roboto Bold (SIL OFL 1.1) instead.

Attribution for both lives in `THIRD_PARTY_LICENSES` and `NOTICE`. Keep them
in sync when touching the renderer or the bundled font.

### Embedded Font

`assets/fonts/Roboto-Bold-subset.ttf` is Roboto Bold subset to the 54
characters in `BASIC_CHAR` — 8,196 bytes instead of 33,864.

**The subset locks the character set.** Adding a character to `BASIC_CHAR`
without regenerating the font makes it render as `.notdef`. The
`test_every_basic_char_has_a_glyph` unit test fails when the two drift, so
follow it up with:

```bash
pip install fonttools
python3 scripts/subset-font.py path/to/Roboto-Bold.ttf
```

Roboto declares no Reserved Font Name, so the subset keeps the family name
and needs no renaming. Swapping to a font that *does* reserve its name (most
OFL fonts, including Liberation) would reintroduce that obligation.

## Background Tasks

### Cleanup Task

Runs every 60 seconds (configurable via `CLEANUP_INTERVAL_SECONDS`):
- Deletes sessions where `expires_at < current_time`
- Logs number of sessions cleaned up
- Runs asynchronously without blocking the server

## Error Handling

All errors return JSON responses with this format:

```json
{
  "error": "error_code",
  "message": "Human-readable description"
}
```

### Common Error Codes

- `session_not_found` - Session doesn't exist or expired (404)
- `unauthorized` - Invalid or missing API key (401)
- `invalid_parameters` - Bad request parameters (400)
- `database_error` - Internal database error (500)
- `internal_error` - Other internal errors (500)

## Logging

The application uses structured logging via `tracing`:

```bash
# Set log level via environment variable
RUST_LOG=captchapi=debug,tower_http=debug cargo run

# Log levels: trace, debug, info, warn, error
```

Logs include:
- Server startup and configuration
- Session creation, validation, deletion
- API key usage (last_used_at updates)
- Cleanup task operations
- Errors and warnings

## Performance Characteristics

- **Database**: SQLite in-process (no network overhead)
- **Async**: Full async/await throughout
- **Connection Pooling**: Max 5 concurrent database connections
- **Memory**: Low footprint (no image caching, base64 on-demand)
- **Cleanup**: Periodic background task (non-blocking)

## Future Enhancements

Ideas not yet scheduled:

- Multiple CAPTCHA types (math, audio)
- Session statistics/analytics
- Horizontal scaling support
- OpenAPI/Swagger documentation
- Admin dashboard

## Troubleshooting

### Database Issues

```bash
# Delete database to reset
rm -rf data/

# Restart server (will recreate database)
cargo run
```

### API Key Issues

API keys are managed via the admin API endpoints (require master key):

```bash
# Create a new API key
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "My Key"}'

# List all keys
curl http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY"

# Deactivate a key
curl -X PUT http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"is_active": false}'

# Delete a key
curl -X DELETE http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_KEY"
```

See `docs/API.md` for full API key management documentation.

### Port Already in Use

```bash
# Change port in .env
SERVER_PORT=8080

# Or use environment variable
SERVER_PORT=8080 cargo run
```

## Contributing

When making changes, follow this workflow:

### 1. Write Tests First
- ✅ Write Rust unit tests for new functionality
- ✅ Write Rust integration tests for HTTP endpoints
- ✅ Write Bruno API tests for new endpoints (in `.bruno/Tests/`)
- ✅ Add Bruno core endpoints for normal usage (in `.bruno/`)

### 2. Implement Changes
- ✅ Write the actual code
- ✅ Update relevant documentation (README.md, docs/, this file)

### 3. Verify Quality (Run IN ORDER)
- ✅ Step 1: Run `cargo fmt` to format code
- ✅ Step 2: Run `cargo clippy` to check for issues
- ✅ Step 3: Run `cargo check` to verify compilation
- ✅ Step 4: Run `cargo nextest run` to verify Rust tests pass
- ✅ Step 5: Run `./.bruno/Tests/Scripts/test-bruno-full.sh` to verify API tests (requires running server)

### 4. Finalize
- ✅ Commit with descriptive messages

**CRITICAL REQUIREMENTS:**

1. **All 5 verification steps must pass** before committing
2. **Both test suites are mandatory** - Changes may pass Rust tests but break the HTTP API
3. **Write tests BEFORE implementing features** - Test-driven development
4. **Update BOTH test suites** when making changes:
   - Rust tests (unit + integration)
   - API tests (Bruno collection)
5. **Server must be running** for API tests (Step 5)

### Test Coverage for New Features

Every new endpoint MUST have:
- ✅ Rust integration tests (success + failure cases)
- ✅ Bruno test scenarios (in `.bruno/Tests/`)
- ✅ Bruno core endpoint (in `.bruno/` root)
- ✅ Documentation in relevant files

## License

Apache-2.0

## Support

For issues, questions, or contributions, please refer to the project repository.

---

**Last Updated**: 2026-03-03
**Version**: 1.0.0
**Rust Edition**: 2021
