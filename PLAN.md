# CaptchAPI - Implementation Plan

## Overview
CaptchAPI is a REST API service for creating, validating, and consuming CAPTCHA challenges. It provides a simple, secure way to integrate CAPTCHA verification into applications with persistent storage and flexible configuration.

## Technology Stack

### CAPTCHA Generation: captcha-rs v0.2.11
- **Dependencies** (all recent/current):
  - `base64 ^0.21.0` - Base64 encoding
  - `image ^0.24.5` - Image processing
  - `imageproc ^0.23.0` - Image manipulation
  - `rand ^0.8.5` - Random number generation
  - `rusttype ^0.9.2` - Font rendering
  - `log ^0.4.16` - Logging facade

- **Features**: Modern dependencies, built-in base64 encoding, complexity controls (1-10), dark mode support

### Web Framework: Axum v0.8
- Async-first architecture built on Tokio runtime
- Ergonomic API with excellent type safety
- Powerful middleware/layer system for authentication
- Lowest memory footprint among top Rust frameworks
- Active development and well-maintained

### Storage: SQLx + SQLite
- Full async support with Tokio integration
- Compile-time SQL query checking
- Built-in connection pooling
- In-process embedded database (no external dependencies)
- Battle-tested reliability

### Additional Dependencies
- `tokio` - Async runtime
- `serde` + `serde_json` - Serialization
- `uuid` - Session ID generation
- `tower` / `tower-http` - Middleware infrastructure
- `sha2` + `hex` - API key hashing
- `chrono` - Timestamp handling
- `tracing` - Structured logging

---

## API Specification

### Base URL
```
/api/v1/sessions
```

### Authentication
- **Protected Endpoints**: Bearer token authentication via `Authorization: Bearer <api_key>` header
- **Public Endpoints**: Image retrieval (requires valid session ID)

---

## Endpoints

### 1. Create Session (Protected)
Create a new CAPTCHA session with optional custom parameters.

```http
POST /api/v1/sessions
Authorization: Bearer <api_key>
Content-Type: application/json

Request Body:
{
  "text": "ABCD5",              // Optional: custom text (default: random)
  "expires_in_seconds": 300,    // Optional: TTL (default: 300)
  "difficulty": 5,              // Optional: 1-10 (default: 5)
  "width": 220,                 // Optional: pixels (default: 220)
  "height": 120,                // Optional: pixels (default: 120)
  "dark_mode": false            // Optional: theme (default: false)
}

Response: 201 Created
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "expires_at": "2025-10-23T12:35:00Z",
  "created_at": "2025-10-23T12:30:00Z"
}
```

### 2. Get CAPTCHA Image (Public)
Retrieve the CAPTCHA image for a session (base64 encoded).

```http
GET /api/v1/sessions/{session_id}/image

Response: 200 OK
Content-Type: application/json
{
  "image": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg...",
  "expires_at": "2025-10-23T12:35:00Z"
}

Error: 404 Not Found
{
  "error": "session_not_found",
  "message": "Session does not exist or has expired"
}
```

### 3. Validate Solution (Protected)
Validate a user's solution to the CAPTCHA.

```http
POST /api/v1/sessions/{session_id}/validate
Authorization: Bearer <api_key>
Content-Type: application/json

Request Body:
{
  "solution": "ABCD5"
}

Response: 200 OK
{
  "valid": true,
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}

Note: Session is automatically deleted after successful validation or 3 failed attempts
```

### 4. Delete Session (Protected)
Prematurely delete a CAPTCHA session.

```http
DELETE /api/v1/sessions/{session_id}
Authorization: Bearer <api_key>

Response: 204 No Content

Error: 404 Not Found
```

### 5. Health Check (Public)
Check service health status.

```http
GET /health

Response: 200 OK
{
  "status": "healthy",
  "version": "0.1.0"
}
```

---

## Database Schema

### Sessions Table
Stores active CAPTCHA sessions with their metadata and solutions.

```sql
CREATE TABLE sessions (
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

CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX idx_sessions_created_at ON sessions(created_at);
```

### API Keys Table
Stores hashed API keys for authentication.

```sql
CREATE TABLE api_keys (
    key_hash TEXT PRIMARY KEY,        -- SHA256(api_key)
    description TEXT,                 -- Human-readable label
    created_at INTEGER NOT NULL,      -- Unix timestamp
    last_used_at INTEGER,             -- Unix timestamp
    is_active INTEGER DEFAULT 1       -- Boolean: 0=disabled, 1=active
);

CREATE INDEX idx_api_keys_active ON api_keys(is_active);
```

---

## Project Structure

```
captchapi/
├── Cargo.toml
├── .env                         # Configuration (not committed)
├── .gitignore
├── PLAN.md                      # This document
├── PROGRESS.md                  # Development progress tracking
├── migrations/                  # SQLx migrations
│   └── 20250101000000_init.sql
├── src/
│   ├── main.rs                  # Entry point, server setup
│   ├── config.rs                # Environment configuration
│   ├── error.rs                 # Error types and handling
│   ├── models/
│   │   ├── mod.rs
│   │   ├── session.rs           # Session struct, conversions
│   │   └── api_key.rs           # API key struct
│   ├── services/
│   │   ├── mod.rs
│   │   ├── captcha.rs           # captcha-rs integration
│   │   ├── storage.rs           # Database operations
│   │   └── auth.rs              # API key validation
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── sessions.rs          # Session CRUD handlers
│   │   └── health.rs            # Health check
│   ├── middleware/
│   │   ├── mod.rs
│   │   └── auth.rs              # Bearer token middleware
│   └── tasks/
│       ├── mod.rs
│       └── cleanup.rs           # Expired session cleanup
└── tests/
    └── integration_tests.rs
```

---

## Configuration

### Environment Variables (.env)

```bash
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:./data/captchapi.db

# Security
API_KEY_SALT=your-random-salt-here

# CAPTCHA defaults
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3

# Cleanup task
CLEANUP_INTERVAL_SECONDS=60
```

---

## Implementation Steps

### Phase 1: Foundation
1. ✓ Project initialization
2. Add dependencies to Cargo.toml
3. Create directory structure
4. Set up database migrations
5. Implement configuration module
6. Implement error handling module

### Phase 2: Core Services
7. Implement data models (Session, ApiKey)
8. Implement storage service with CRUD operations
9. Implement CAPTCHA service (captcha-rs integration)
10. Implement authentication service

### Phase 3: API Layer
11. Implement authentication middleware
12. Implement session creation endpoint
13. Implement image retrieval endpoint
14. Implement validation endpoint
15. Implement deletion endpoint
16. Add health check endpoint

### Phase 4: Background Tasks & Polish
17. Implement cleanup task for expired sessions
18. Update main.rs with complete server setup
19. Add comprehensive error handling
20. Write integration tests
21. Add logging and tracing

---

## Security Features

### API Key Management
- Keys hashed with SHA256 before storage
- Never store plaintext keys
- Track last usage timestamp
- Support for key activation/deactivation

### Session Security
- Automatic deletion after successful validation
- Limit validation attempts (max 3)
- Automatic expiration based on TTL
- Case-insensitive solution comparison

### Input Validation
- Sanitize all user inputs
- Enforce maximum TTL limits
- Validate complexity ranges (1-10)
- Validate image dimensions

### Rate Limiting (Future Enhancement)
- Per-IP limits on public image endpoint
- Per-API-key limits on session creation

### CORS Configuration
- Configurable allowed origins
- Proper security headers

---

## Key Features

✓ **In-process storage** - No external database server required
✓ **Async throughout** - Full Tokio async/await support
✓ **Type-safe queries** - Compile-time SQL checking with sqlx
✓ **Flexible CAPTCHA** - Configurable difficulty, size, theme
✓ **RESTful API** - Clean, predictable endpoint design
✓ **Auto-cleanup** - Background task removes expired sessions
✓ **Production-ready** - Comprehensive error handling, logging, security
✓ **Modern dependencies** - All crates are current and well-maintained

---

## Testing Strategy

### Integration Tests
- Test all API endpoints
- Test authentication flow
- Test session lifecycle (create → retrieve → validate)
- Test session expiration
- Test validation attempt limits
- Test error responses

### Manual Testing
- Verify image generation with different parameters
- Test concurrent requests
- Verify cleanup task execution
- Test with invalid API keys

---

## Future Enhancements

### Potential Features
- Rate limiting per IP/API key
- Metrics and monitoring endpoints
- Multiple CAPTCHA types (math, audio, etc.)
- Custom font support
- Batch session creation
- Session statistics and analytics
- OpenAPI/Swagger documentation
- Docker containerization
- Horizontal scaling support

---

## Dependencies Reference

See Cargo.toml for complete dependency list with versions.

Key dependencies:
- **axum**: Web framework
- **tokio**: Async runtime
- **sqlx**: Database operations
- **captcha-rs**: CAPTCHA generation
- **serde**: Serialization
- **uuid**: Session ID generation
- **sha2**: Cryptographic hashing
- **tracing**: Structured logging
