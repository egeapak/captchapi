[![CI](https://github.com/egeapak/captchapi/actions/workflows/ci.yml/badge.svg)](https://github.com/egeapak/captchapi/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

# CaptchAPI

A secure, high-performance REST API for CAPTCHA generation and validation. Built with Rust, featuring async/await architecture, SQLite persistence, and distroless Docker containers.

CAPTCHA images are generated using [captcha-rs](https://github.com/samirdjelal/captcha-rs) with configurable difficulty and dark mode support:

| Easy (difficulty 2) | Hard + dark mode (difficulty 8) |
|:---:|:---:|
| ![Easy CAPTCHA](docs/images/captcha-easy.jpeg) | ![Hard CAPTCHA](docs/images/captcha-hard-dark.jpeg) |

## Use Cases

**Web Application Protection**
Protect sign-up forms, login pages, and contact forms from automated bot submissions while maintaining a smooth user experience.

**API Rate Limiting Enhancement**
Add human verification as an additional layer on top of rate limiting for sensitive endpoints like password resets or account creation.

**Microservices Architecture**
Deploy as a standalone CAPTCHA service that multiple applications can consume via REST API, centralizing CAPTCHA logic and reducing code duplication.

## Features

**Security First**
- API key authentication with SHA256 hashing
- Nonroot Docker containers (UID 65532)
- Distroless base image with minimal attack surface
- Automatic session expiration and cleanup
- Configurable validation attempt limits

**High Performance**
- Async/await throughout (Tokio + Axum)
- In-process SQLite (zero network overhead)
- Connection pooling (max 5 concurrent)
- Compact Docker image with distroless base
- Low memory footprint

**Developer Friendly**
- Full REST API with JSON responses
- Configurable CAPTCHA difficulty (1-10)
- Dark mode support
- Custom dimensions and compression
- Comprehensive test coverage
- Complete API documentation

**Production Ready**
- Three Docker deployment modes (transient, volume, bind mount)
- Structured logging with tracing
- Health check endpoint
- Background cleanup tasks
- Per-IP rate limiting

## Quick Start

**Prerequisites:**
- Rust 1.80+ and Cargo
- SQLite support (usually built-in)

**Development Setup:**

```bash
# 1. Clone the repository
git clone https://github.com/egeapak/captchapi
cd captchapi

# 2. Copy environment template
cp .env.example .env

# 3. Generate secure keys (optional for development)
# API_KEY_SALT=$(openssl rand -base64 32)
# MASTER_API_KEY=$(openssl rand -base64 32)
# Update these values in .env

# 4. Run the application
cargo run

# 5. Verify it's running
curl http://localhost:3000/health
```

The server will start on `http://localhost:3000` with automatic database initialization.

## Docker Deployment

### Pre-built Image

Multi-platform images (amd64 + arm64) are published to GitHub Container Registry:

```bash
docker pull ghcr.io/egeapak/captchapi:latest
```

```bash
# Transient (no volume)
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
  ghcr.io/egeapak/captchapi:latest

# Production (with volume)
docker volume create captchapi-data
docker run -p 3000:3000 \
  -v captchapi-data:/data \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
  ghcr.io/egeapak/captchapi:latest
```

### Building from Source

**Prerequisites:**
- [cross](https://github.com/cross-rs/cross): `cargo install cross`
- [just](https://github.com/casey/just): `cargo install just`

```bash
# Build optimized static image
just

# Run transient (testing)
just run

# Run with persistent volume (production)
just run-volume
```

See `docker/README.md` for detailed Docker build documentation and bind mount instructions.

## Usage Guide

### Complete Workflow

**Step 1: Create an API Key** (one-time setup)

```bash
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "description": "Frontend Application"
  }'
```

Response:
```json
{
  "api_key": "generated-api-key-value",
  "key_hash": "hash-value",
  "description": "Frontend Application",
  "created_at": "2025-01-15T10:30:00Z"
}
```

Save the `api_key` value - it's only shown once.

---

**Step 2: Create a CAPTCHA Session**

```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "length": 5,
    "difficulty": 5,
    "expires_in_seconds": 300,
    "dark_mode": false
  }'
```

Response:
```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2025-01-15T10:35:00Z",
  "expires_at": "2025-01-15T10:40:00Z"
}
```

---

**Step 3: Display the CAPTCHA Image**

Use the session ID to display the CAPTCHA directly in HTML:

```html
<img src="http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg" />
```

Or download it:
```bash
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg \
  -o captcha.jpeg
```

---

**Step 4: Validate User's Solution**

```bash
curl -X POST http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/validate \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "solution": "aBc5X"
  }'
```

Success response (session auto-deleted):
```json
{
  "valid": true,
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

Failure response (attempt count incremented):
```json
{
  "valid": false,
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Important Notes:**
- Validation is **case-sensitive**: "aBc5X" ≠ "abc5x"
- After 3 failed attempts, the session is automatically deleted

---

### Error Handling

All errors return JSON with standardized format:

```json
{
  "error": "session_not_found",
  "message": "Session not found or has expired"
}
```

Common error codes:
- `unauthorized` (401) - Invalid or missing API key
- `session_not_found` (404) - Session doesn't exist or expired
- `invalid_parameters` (400) - Bad request parameters
- `internal_error` (500) - Server error

---

### Admin Operations

Admin endpoints require the master API key.

**API Key Management:**
```bash
# List all keys
curl http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY"

# Deactivate a key
curl -X PUT http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"is_active": false}'

# Delete a key
curl -X DELETE http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY"
```

**Manual Cleanup:**
```bash
curl -X POST http://localhost:3000/api/v1/admin/cleanup \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY"
```

Cleanup also runs automatically in the background every 60 seconds.

For the complete API reference, see [`docs/API.md`](docs/API.md).

## Testing & Development

### Running Tests

**Rust Tests** (uses [cargo-nextest](https://nexte.st/) for process-per-test isolation)

```bash
# Run all tests
cargo nextest run

# Run only unit tests
cargo nextest run --lib

# Run only integration tests
cargo nextest run --test sessions_test
```

**API Tests with Bruno**

Requires the server to be running first.

```bash
# Terminal 1: Start the server
cargo run

# Terminal 2: Run API tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

### Development Workflow

After every code change, run these commands **in order**:

```bash
# 1. Format code
cargo fmt

# 2. Run linter
cargo clippy

# 3. Check compilation
cargo check

# 4. Run Rust tests
cargo nextest run

# 5. Run API tests (requires running server in another terminal)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

All five steps must pass before committing changes.

## Contributing

We welcome contributions! Please follow these guidelines:

1. **Write tests first** - Add both Rust tests and Bruno API tests for new features
2. **Run the complete quality checklist** - All 5 development workflow steps must pass
3. **Update documentation** - If adding features, update relevant documentation
4. **Follow existing patterns** - Study the codebase structure before adding new code
5. **Keep commits focused** - One logical change per commit with clear messages

---

**Rust Edition**: 2021
**License**: Apache-2.0
