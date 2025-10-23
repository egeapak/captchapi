# CaptchAPI - Project Documentation

## Overview

CaptchAPI is a REST API service for creating, validating, and consuming CAPTCHA challenges. Built with Rust, it provides a secure, high-performance solution for integrating CAPTCHA verification into applications.

## Architecture

### Technology Stack

- **Language**: Rust (Edition 2021)
- **Web Framework**: Axum 0.8 (async-first, built on Tokio)
- **Database**: SQLite via SQLx 0.8 (async, compile-time checked queries)
- **CAPTCHA Generation**: captcha-rs 0.2.11
- **Authentication**: API key-based with SHA256 hashing

### Key Features

- ✅ Configurable CAPTCHA generation (difficulty 1-10, dark mode, custom dimensions)
- ✅ API key authentication for protected operations
- ✅ Public image retrieval (requires session ID)
- ✅ Automatic session expiration and cleanup
- ✅ Validation attempt limiting (max 3 attempts)
- ✅ Case-insensitive solution matching
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

## API Endpoints

### Public Endpoints

- `GET /health` - Health check
- `GET /api/v1/sessions/:id/image` - Retrieve CAPTCHA image (base64)

### Protected Endpoints (Require API Key)

- `POST /api/v1/sessions` - Create new CAPTCHA session
- `POST /api/v1/sessions/:id/validate` - Validate user solution
- `DELETE /api/v1/sessions/:id` - Delete session

### Request/Response Examples

#### Create Session
```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "difficulty": 5,
    "expires_in_seconds": 300,
    "width": 220,
    "height": 120,
    "dark_mode": false
  }'

# Response:
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "expires_at": "2025-10-23T12:35:00Z",
  "created_at": "2025-10-23T12:30:00Z"
}
```

#### Get CAPTCHA Image
```bash
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image

# Response:
{
  "image": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg...",
  "expires_at": "2025-10-23T12:35:00Z"
}
```

#### Validate Solution
```bash
curl -X POST http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/validate \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"solution": "ABC123"}'

# Response:
{
  "valid": true,
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

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

# CAPTCHA Defaults
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60
```

**Important**: Always change `API_KEY_SALT` to a random string in production!

## Database Schema

### Sessions Table
Stores active CAPTCHA sessions with metadata and solutions.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution TEXT NOT NULL,           -- Correct answer (lowercase)
    image_base64 TEXT NOT NULL,       -- Base64 encoded PNG
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

**IMPORTANT**: After **EVERY** code change, you **MUST** run:

```bash
# 1. Format code
cargo fmt

# 2. Run linter
cargo clippy

# 3. Check compilation
cargo check
```

These steps ensure:
- ✅ Consistent code formatting
- ✅ No common mistakes or anti-patterns
- ✅ Code compiles successfully

### Testing

```bash
# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run

# Build release binary
cargo build --release
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

When making changes:

1. ✅ Update relevant documentation (PLAN.md, PROGRESS.md, this file)
2. ✅ Run `cargo fmt` to format code
3. ✅ Run `cargo clippy` to check for issues
4. ✅ Run `cargo check` to verify compilation
5. ✅ Test your changes
6. ✅ Update PROGRESS.md with completed tasks
7. ✅ Commit with descriptive messages

## License

[Specify your license here]

## Support

For issues, questions, or contributions, please refer to the project repository.

---

**Last Updated**: 2025-10-23
**Version**: 0.1.0
**Rust Edition**: 2021
