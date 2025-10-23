# CaptchAPI

A secure, high-performance REST API for creating, validating, and consuming CAPTCHA challenges. Built with Rust, featuring distroless Docker containers and nonroot security.

## Features

- ✅ **Secure**: Runs as nonroot (UID 65532) in distroless containers
- ✅ **Fast**: Rust-powered with async/await throughout
- ✅ **Flexible**: Three persistence modes (transient, volume, bind mount)
- ✅ **Complete**: Full API key management and session handling
- ✅ **Production-Ready**: Comprehensive testing, documentation, and deployment options

## Quick Start

### Development (Cargo)

```bash
# Copy environment template
cp .env.example .env

# Run the application
cargo run

# Run tests
cargo test
```

### Production (Docker)

```bash
# Navigate to docker directory
cd docker

# Start with Docker Compose
docker-compose up -d

# Check health
curl http://localhost:3000/health
```

## Documentation

- **[Project Overview](.claude/CLAUDE.md)** - Architecture, development workflow, and project structure
- **[API Documentation](.claude/docs/API.md)** - Complete endpoint reference and usage examples
- **[Testing Guide](.claude/docs/TESTING.md)** - Test structure, writing tests, and debugging
- **[Docker Usage](docker/DOCKER_USAGE.md)** - Docker deployment with all persistence options
- **[Docker Summary](docker/DOCKER_SUMMARY.md)** - Implementation details and architecture

## Project Structure

```
captchapi/
├── .claude/                    # Project and API documentation
│   ├── CLAUDE.md              # Project overview
│   └── docs/
│       ├── API.md             # API reference
│       └── TESTING.md         # Testing guide
├── docker/                     # Docker configuration
│   ├── Dockerfile             # Multi-stage distroless build
│   ├── docker-compose.yml     # Default (volume mode)
│   ├── docker-compose.*.yml   # Alternative configurations
│   ├── DOCKER_USAGE.md        # Complete Docker guide
│   └── DOCKER_SUMMARY.md      # Implementation details
├── migrations/                 # SQLx database migrations
├── src/                        # Application source code
│   ├── main.rs                # Entry point
│   ├── config.rs              # Environment configuration
│   ├── error.rs               # Error handling
│   ├── models/                # Data structures
│   ├── services/              # Business logic
│   ├── routes/                # HTTP endpoints
│   ├── middleware/            # Authentication
│   └── tasks/                 # Background jobs
├── tests/                      # Integration tests
├── Cargo.toml                  # Dependencies
└── .env.example               # Environment template
```

## API Endpoints

### Public
- `GET /health` - Health check
- `GET /api/v1/sessions/{id}/image` - Get CAPTCHA (JSON)
- `GET /api/v1/sessions/{id}/image.jpeg` - Get CAPTCHA (binary)

### Protected (Require API Key)
- `POST /api/v1/sessions` - Create CAPTCHA session
- `POST /api/v1/sessions/{id}/validate` - Validate solution
- `DELETE /api/v1/sessions/{id}` - Delete session

### Admin (Require Master Key)
- `POST /api/v1/api-keys` - Create API key
- `GET /api/v1/api-keys` - List API keys
- `PUT /api/v1/api-keys/{key_hash}` - Update API key
- `DELETE /api/v1/api-keys/{key_hash}` - Delete API key

## Environment Variables

```bash
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:./data/captchapi.db

# Security (CHANGE IN PRODUCTION!)
API_KEY_SALT=your-random-salt
MASTER_API_KEY=your-master-key

# CAPTCHA Configuration
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60
```

## Technology Stack

- **Language**: Rust (Edition 2021)
- **Web Framework**: Axum 0.8
- **Database**: SQLite via SQLx 0.8
- **CAPTCHA**: captcha-rs 0.2.11
- **Container**: Distroless (nonroot)

## Security

- ✅ Nonroot containers (UID 65532)
- ✅ Distroless base image (no shell, minimal attack surface)
- ✅ API key authentication with SHA256 hashing
- ✅ Automatic session expiration and cleanup
- ✅ Rate limiting via validation attempts
- ✅ Final Docker image: 32.9MB

## Development

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Run tests
cargo test

# Run specific test
cargo test test_name

# Build release
cargo build --release
```

## License

[Specify your license]

## Contributing

See [CLAUDE.md](.claude/CLAUDE.md) for development guidelines and project architecture.

---

**Version**: 0.1.0
**Rust Edition**: 2021
**Docker**: Multi-stage with distroless runtime
