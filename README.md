[![CI](https://github.com/egeapak/captchapi/actions/workflows/ci.yml/badge.svg)](https://github.com/egeapak/captchapi/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

# CaptchAPI

A secure, high-performance REST API for CAPTCHA generation and validation. Built with Rust, featuring async/await architecture, SQLite persistence, and distroless Docker containers.

CAPTCHA images are rendered in-process with configurable difficulty and dark mode support:

| Easy (difficulty 2) | Hard + dark mode (difficulty 8) |
|:---:|:---:|
| ![Easy CAPTCHA](docs/images/captcha-easy.jpeg) | ![Hard CAPTCHA](docs/images/captcha-hard-dark.jpeg) |

## Table of Contents

- [Features](#features)
- [Quick Start](#quick-start)
- [Docker Deployment](#docker-deployment)
- [Usage](#usage)
- [Configuration](#configuration)
- [API Reference](#api-reference)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

## Features

- **Security First**
  - API key authentication with SHA256 hashing
  - Nonroot Docker containers (UID 65532)
  - Distroless base image with minimal attack surface
  - Automatic session expiration and cleanup
  - Configurable validation attempt limits

- **High Performance**
  - Async/await throughout (Tokio + Axum)
  - In-process SQLite (zero network overhead)
  - Connection pooling (max 5 concurrent)
  - Low memory footprint

- **Developer Friendly**
  - Full REST API with JSON responses
  - Configurable CAPTCHA difficulty (1-10), dimensions, and compression
  - Dark mode support
  - Complete API documentation

- **Production Ready**
  - Multi-platform Docker images (amd64 + arm64)
  - Three deployment modes (transient, volume, bind mount)
  - Structured logging with tracing
  - OpenTelemetry integration (optional)
  - Per-IP rate limiting (GCRA algorithm)
  - Background session cleanup

## Quick Start

### Prerequisites

- **Rust 1.85+** and Cargo
- **SQLite** support (usually built-in)

### Setup

```bash
# Clone and enter the project
git clone https://github.com/egeapak/captchapi
cd captchapi

# Copy environment template
cp .env.example .env

# Generate secure keys (recommended)
# API_KEY_SALT=$(openssl rand -base64 32)
# MASTER_API_KEY=$(openssl rand -base64 32)
# Update these values in .env

# Run the application
cargo run

# Verify it's running
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

**Prerequisites:** [cross](https://github.com/cross-rs/cross) and [just](https://github.com/casey/just)

```bash
# Build optimized static image
just

# Run transient (testing)
just run

# Run with persistent volume (production)
just run-volume
```

See [`docker/README.md`](docker/README.md) for detailed build documentation and bind mount instructions.

## Usage

### 1. Create an API Key (one-time setup)

```bash
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "Frontend Application"}'
```

Save the returned `api_key` value — it's only shown once.

### 2. Create a CAPTCHA Session

```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"length": 5, "difficulty": 5, "expires_in_seconds": 300}'
```

### 3. Display the CAPTCHA Image

```html
<img src="http://localhost:3000/api/v1/sessions/{session_id}/image.jpeg" />
```

### 4. Validate User's Solution

```bash
curl -X POST http://localhost:3000/api/v1/sessions/{session_id}/validate \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"solution": "aBc5X"}'
```

- Validation is **case-sensitive**: `aBc5X` ≠ `abc5x`
- Session is **auto-deleted** after successful validation
- After **3 failed attempts**, the session is automatically deleted

### Error Handling

All errors return JSON with a standardized format:

```json
{
  "error": "session_not_found",
  "message": "Session not found or has expired"
}
```

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Invalid or missing API key |
| `session_not_found` | 404 | Session doesn't exist or expired |
| `invalid_parameters` | 400 | Bad request parameters |
| `internal_error` | 500 | Server error |

### Admin Operations

Admin endpoints require the master API key.

```bash
# List all API keys
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

# Manual cleanup of expired sessions
curl -X POST http://localhost:3000/api/v1/admin/cleanup \
  -H "Authorization: Bearer YOUR_MASTER_API_KEY"
```

## Configuration

All configuration is via environment variables. Create a `.env` file based on `.env.example`.

### Server

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `SERVER_HOST` | Bind address | `0.0.0.0` | No |
| `SERVER_PORT` | Listen port | `3000` | No |

### Database

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `DATABASE_URL` | SQLite connection string | `sqlite:./data/captchapi.db` | No |
| `DATABASE_MAX_CONNECTIONS` | Connection pool size | `5` | No |

### Security

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `API_KEY_SALT` | Salt for API key hashing (min 16 chars) | - | **Yes** |
| `MASTER_API_KEY` | Admin API key (min 16 chars) | - | **Yes** |

### CAPTCHA

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `DEFAULT_SESSION_TTL_SECONDS` | Default session expiration | `300` | No |
| `MAX_SESSION_TTL_SECONDS` | Maximum allowed TTL | `3600` | No |
| `MAX_VALIDATION_ATTEMPTS` | Failed attempts before deletion | `3` | No |
| `CAPTCHA_COMPRESSION` | JPEG quality (1-100) | `40` | No |

### Rate Limiting

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `RATE_LIMIT_REQUESTS_PER_SECOND` | Sustained request rate per IP | `2` | No |
| `RATE_LIMIT_BURST_SIZE` | Burst capacity per IP | `10` | No |
| `RATE_LIMIT_REVERSE_PROXY` | Read client IP from proxy headers | `false` | No |

> **Warning:** Only enable `RATE_LIMIT_REVERSE_PROXY` if you trust your proxy — clients can spoof headers otherwise.

### Background Tasks

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `CLEANUP_INTERVAL_SECONDS` | Expired session cleanup interval | `60` | No |

### OpenTelemetry (optional)

OpenTelemetry trace export is **not compiled in by default**. The OTLP exporter
pulls in a full HTTP client worth roughly 700 KB of binary, so it sits behind
the `otel` cargo feature:

```bash
cargo build --release --features otel
```

The variables below apply to a binary built with that feature. A binary built
without it warns on startup if `OTEL_ENABLED` is set, rather than ignoring it
silently. Prometheus-style metrics are unaffected and always available.

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `OTEL_ENABLED` | Enable OpenTelemetry tracing (requires the `otel` feature) | `false` | No |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP HTTP endpoint | `http://localhost:4318` | No |
| `OTEL_SERVICE_NAME` | Service name for traces | `captchapi` | No |

## API Reference

For the complete API documentation including all endpoints, request/response formats, and usage examples, see [`docs/API.md`](docs/API.md).

### Endpoints Overview

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| `GET` | `/health` | None | Health check |
| `POST` | `/api/v1/sessions` | API Key | Create CAPTCHA session |
| `GET` | `/api/v1/sessions/{id}` | None | Get session details |
| `GET` | `/api/v1/sessions/{id}/image.jpeg` | None | Get CAPTCHA image |
| `POST` | `/api/v1/sessions/{id}/validate` | API Key | Validate solution |
| `DELETE` | `/api/v1/sessions/{id}` | API Key | Delete session |
| `POST` | `/api/v1/api-keys` | Master | Create API key |
| `GET` | `/api/v1/api-keys` | Master | List API keys |
| `PUT` | `/api/v1/api-keys/{hash}` | Master | Update API key |
| `DELETE` | `/api/v1/api-keys/{hash}` | Master | Delete API key |
| `POST` | `/api/v1/admin/cleanup` | Master | Manual cleanup |

## Development

### Building

```bash
cargo build           # Debug build
cargo build --release # Release build
```

### Testing

This project uses [cargo-nextest](https://nexte.st/) for process-per-test isolation.

```bash
# Run all tests
cargo nextest run

# Unit tests only
cargo nextest run --lib

# Integration tests only
cargo nextest run --test sessions_test
```

**API Tests** (requires running server):

```bash
# Terminal 1
cargo run

# Terminal 2
./.bruno/Tests/Scripts/test-bruno-full.sh
```

### Code Quality

After every code change, run these commands **in order**:

```bash
cargo fmt          # Format code
cargo clippy       # Run linter
cargo check        # Check compilation
cargo nextest run  # Run tests
```

All steps must pass before committing.

## Contributing

Please see [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, testing, and pull request guidelines.

## License

This project is licensed under the [Apache-2.0 License](LICENSE).
