[![CI](https://github.com/egeapak/captchapi/actions/workflows/ci.yml/badge.svg)](https://github.com/egeapak/captchapi/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

# CaptchAPI

A secure, high-performance REST API for CAPTCHA generation and validation. Built with Rust, featuring async/await architecture, SQLite persistence, and distroless Docker containers.

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
- Custom dimensions
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

**Optimized Static Image: 7.42 MB** 🚀

CaptchAPI uses a highly optimized static musl binary with distroless base image, achieving one of the smallest Rust web service images possible.

### Quick Build

**Prerequisites:**
- [cross](https://github.com/cross-rs/cross): `cargo install cross`
- [just](https://github.com/casey/just): `cargo install just`

**Build & Run:**

```bash
# Build optimized static image (7.42 MB)
just

# Run transient (testing)
just run

# Run with persistent volume (production)
just run-volume
```

### Manual Docker Usage

```bash
# Transient (no volume)
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
  captchapi:latest

# Production (with volume)
docker volume create captchapi-data
docker run -p 3000:3000 \
  -v captchapi-data:/data \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
  captchapi:latest
```

See `docker/README.md` for detailed Docker build documentation, bind mount instructions, and CI/CD setup.

### Quick Docker Start

```bash
# 1. Navigate to docker directory
cd docker

# 2. Update environment variables in docker-compose.yml
# Change API_KEY_SALT and MASTER_API_KEY to secure random values

# 3. Start with volume mode (recommended)
docker-compose up -d

# 4. Check health
curl http://localhost:3000/health
```

For detailed Docker documentation including Kubernetes deployment, troubleshooting, and backup strategies, see `docker/README.md` in the repository.

## Node.js SDK

Official Node.js packages are available for easy integration:

- **`@captchapi/core`** - Framework-agnostic core client for CaptchAPI
- **`captchapi`** - Full-featured Node.js SDK with additional helpers

Install via npm:

```bash
npm install @captchapi/core
# or
npm install captchapi
```

## Usage Guide

### Breaking Changes in v1.0.0

The `text` field has been removed from the `CreateSessionResponse`. Previously, the session creation response included the CAPTCHA solution in plain text. This was a security risk and has been removed in v1.0.0. To display the CAPTCHA to users, retrieve the image via the image endpoints and have users read and submit the solution themselves.

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

**Step 3: Retrieve the CAPTCHA Image**

**Get session details (metadata only)**
```bash
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000
```

Response:
```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2025-01-15T10:35:00Z",
  "expires_at": "2025-01-15T10:40:00Z",
  "attempt_count": 0,
  "difficulty": 5,
  "width": 220,
  "height": 120,
  "dark_mode": false
}
```

**Get CAPTCHA image as binary JPEG**
```bash
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg \
  -o captcha.jpeg
```

Use this URL directly in HTML:
```html
<img src="http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg" />
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

**Manual Cleanup** (requires master API key)

Manually trigger cleanup of expired sessions:

```bash
curl -X POST http://localhost:3000/api/v1/admin/cleanup \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY"
```

Response:
```json
{
  "sessions_deleted": 5,
  "message": "Successfully cleaned up 5 expired session(s)"
}
```

Note: Cleanup runs automatically in the background every 60 seconds by default. This endpoint is useful for immediate cleanup or testing.

## Testing & Development

### Running Tests

**Rust Tests** (40 tests total)

```bash
# Run all tests
cargo test

# Run only unit tests
cargo test --lib

# Run only integration tests
cargo test --test sessions_test

# Run with output
cargo test -- --nocapture
```

**API Tests with Bruno** (38 tests across 18 requests)

Requires the server to be running first.

```bash
# Terminal 1: Start the server
cargo run

# Terminal 2: Run API tests

# Quick test suite (6 requests, 16 tests)
./.bruno/Tests/Scripts/test-bruno.sh

# Comprehensive test suite (18 requests, 38 tests)
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
cargo test

# 5. Run API tests (requires running server in another terminal)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

All five steps must pass before committing changes.

### Code Quality Standards

- Consistent formatting via `cargo fmt`
- No clippy warnings (`cargo clippy`)
- All tests passing (both Rust and API tests)
- Meaningful commit messages
- Documentation updates for new features

## Contributing

We welcome contributions! Please follow these guidelines:

### Before Submitting a PR

1. **Write tests first** - Add both Rust tests and Bruno API tests for new features
2. **Run the complete quality checklist** - All 5 development workflow steps must pass
3. **Update documentation** - If adding features, update relevant documentation
4. **Follow existing patterns** - Study the codebase structure before adding new code
5. **Keep commits focused** - One logical change per commit with clear messages

### PR Requirements

- All tests passing (Rust + API tests)
- No clippy warnings
- Code formatted with `cargo fmt`
- Documentation updated (if applicable)
- Clear description of changes and motivation
- Reference any related issues

### Testing Requirements

Every new endpoint must include:
- Rust integration tests (success and failure cases)
- Bruno test scenarios in `.bruno/Tests/`
- Bruno core endpoint in `.bruno/` root directory
- Documentation updates for new endpoints

---

**Version**: 1.0.0
**Rust Edition**: 2021
**License**: Apache-2.0
