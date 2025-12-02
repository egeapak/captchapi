# CaptchAPI - Project Documentation

## Overview

CaptchAPI is a REST API service for creating, validating, and consuming CAPTCHA challenges. Built with Rust, it provides a secure, high-performance solution for integrating CAPTCHA verification into applications.

## Documentation

This file provides project overview, architecture, and development workflow. For detailed information:

- **API Documentation**: See `.claude/docs/API.md` for complete endpoint specifications, request/response formats, and usage examples
- **Testing Guide**: See `.claude/docs/TESTING.md` for test structure, writing tests, and debugging

## Architecture

### Technology Stack

- **Language**: Rust (Edition 2021)
- **Web Framework**: Axum 0.8 (async-first, built on Tokio)
- **Database**: SQLite via SQLx 0.8 (async, compile-time checked queries)
- **CAPTCHA Generation**: captcha-rs 0.2.11
- **Authentication**: API key-based with SHA256 hashing

### Key Features

- ✅ Configurable CAPTCHA generation (difficulty 1-10, dark mode, custom dimensions, custom length)
- ✅ API key authentication for protected operations
- ✅ Public image retrieval (requires session ID)
- ✅ Automatic session expiration and cleanup
- ✅ Validation attempt limiting (max 3 attempts)
- ✅ Case-sensitive solution matching (secure validation)
- ✅ Generated text returned in API response
- ✅ Structured logging with tracing
- ✅ In-process SQLite database (zero external dependencies)

## Project Structure

```
captchapi/
├── PLAN.md                      # Implementation specification
├── PROGRESS.md                  # Development progress tracking
├── CLAUDE.md                    # This file - project documentation
├── .env.example                 # Example environment configuration
├── .gitignore                   # Git ignore rules
├── Cargo.toml                   # Rust dependencies
├── migrations/                  # SQLx database migrations
│   └── 20250101000000_init.sql
└── src/
    ├── main.rs                  # Application entry point
    ├── config.rs                # Environment configuration
    ├── error.rs                 # Error types and handling
    ├── models/                  # Data structures
    │   ├── mod.rs
    │   ├── session.rs           # Session models
    │   └── api_key.rs           # API key models
    ├── services/                # Business logic layer
    │   ├── mod.rs
    │   ├── captcha.rs           # CAPTCHA generation
    │   ├── auth.rs              # API key hashing
    │   └── storage.rs           # Database operations
    ├── routes/                  # HTTP endpoints
    │   ├── mod.rs
    │   ├── health.rs            # Health check
    │   └── sessions.rs          # Session CRUD
    ├── middleware/              # HTTP middleware
    │   ├── mod.rs
    │   └── auth.rs              # Authentication
    └── tasks/                   # Background tasks
        ├── mod.rs
        └── cleanup.rs           # Expired session cleanup
```

## API Documentation

For complete API documentation including all endpoints, request/response formats, and usage examples, see `.claude/docs/API.md`

**Quick Reference:**
- **Public**: Health check, Get CAPTCHA images (JSON/binary)
- **Protected**: Create sessions, Validate solutions, Delete sessions
- **Admin**: Manage API keys (create, list, update, delete)

The API documentation includes:
- Full endpoint specifications
- Request/response examples
- Authentication requirements
- Error codes and handling
- Complete usage flows
- Migration guides

## Configuration

### Environment Variables

Create a `.env` file based on `.env.example`:

```bash
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:./data/captchapi.db

# Security
API_KEY_SALT=CHANGE-THIS-TO-A-RANDOM-SALT-IN-PRODUCTION
MASTER_API_KEY=CHANGE-THIS-TO-A-SECURE-MASTER-KEY-IN-PRODUCTION

# CAPTCHA Defaults
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60
```

**Important**:
- Always change `API_KEY_SALT` to a random string in production!
- Always change `MASTER_API_KEY` to a strong, random key in production!
- The master key has full administrative access - protect it carefully!

## Database Schema

### Sessions Table
Stores active CAPTCHA sessions with metadata and solutions.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution TEXT NOT NULL,           -- Correct answer (lowercase)
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
cargo check
```

#### Step 4: Run Rust Tests
```bash
cargo test
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

For comprehensive testing documentation, see `.claude/docs/TESTING.md`

#### Rust Tests

**Quick Start:**
```bash
cargo test                    # Run all 40 tests
cargo test --lib              # Unit tests only
cargo test --test sessions_test   # Integration tests
```

**Test Coverage:**
- 40 total tests (22 unit + 17 integration + 1 migration)
- Full JPEG signature validation
- HTTP headers and caching verification
- Authentication and authorization flows
- Complete user journey testing

#### API Tests (Bruno)

**Prerequisites:** Server must be running.

**Quick Start:**
```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run tests
# Quick happy path (6 requests, 16 tests)
./.bruno/Tests/Scripts/test-bruno.sh

# Comprehensive suite (18 requests, 38 tests)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**API Test Coverage:**
- 38 total tests across 18 requests
- All 10 endpoints (health, API keys, sessions)
- Success scenarios + failure scenarios
- Authentication and authorization
- Error message formatting
- HTTP status codes and headers

#### Complete Test Workflow

**Run all tests (with server):**
```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run all tests
cargo test
./.bruno/Tests/Scripts/test-bruno-full.sh
```

The testing documentation includes:
- Detailed test structure and organization
- How to write new tests
- Test utilities and helpers (TestApp)
- Bruno API test organization
- Debugging failed tests
- Performance benchmarking
- Best practices and patterns

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

### Best Practices

- ✅ Use HTTPS in production
- ✅ Set strong, random `API_KEY_SALT`
- ✅ Implement rate limiting (future enhancement)
- ✅ Monitor for unusual patterns (multiple failed validations)
- ✅ Keep dependencies updated

## Dependencies

### Core Dependencies

```toml
axum = "0.8"                    # Web framework
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "migrate"] }
captcha-rs = "0.2"              # CAPTCHA generation
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
sha2 = "0.10"                   # Hashing
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"                 # Logging
rand = "0.8"                    # Random generation
```

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

See `PLAN.md` for detailed future feature ideas:

- Rate limiting (per IP, per API key)
- Multiple CAPTCHA types (math, audio)
- Session statistics/analytics
- Horizontal scaling support
- OpenAPI/Swagger documentation
- Docker containerization
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

Currently, API keys must be created manually in the database:

```sql
-- Insert an API key (hash of "test-key-123" with salt "my-salt")
INSERT INTO api_keys (key_hash, description, created_at, is_active)
VALUES (
  '<SHA256_HASH>',
  'Test Key',
  strftime('%s', 'now'),
  1
);
```

**Note**: API key management endpoints are planned for future versions.

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
- ✅ Update relevant documentation (PLAN.md, PROGRESS.md, this file)

### 3. Verify Quality (Run IN ORDER)
- ✅ Step 1: Run `cargo fmt` to format code
- ✅ Step 2: Run `cargo clippy` to check for issues
- ✅ Step 3: Run `cargo check` to verify compilation
- ✅ Step 4: Run `cargo test` to verify Rust tests pass
- ✅ Step 5: Run `./.bruno/Tests/Scripts/test-bruno-full.sh` to verify API tests (requires running server)

### 4. Finalize
- ✅ Update PROGRESS.md with completed tasks
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

[Specify your license here]

## Support

For issues, questions, or contributions, please refer to the project repository.

---

**Last Updated**: 2025-10-23
**Version**: 0.1.0
**Rust Edition**: 2021
