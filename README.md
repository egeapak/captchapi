[![CI](https://github.com/egeapak/captchapi/actions/workflows/ci.yml/badge.svg)](https://github.com/egeapak/captchapi/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org)

# CaptchAPI

A self-hosted REST API for CAPTCHA generation and validation. Written in Rust, ships as a
**2.8 MB** distroless container with zero runtime dependencies, and stores nothing in plaintext —
solutions are keyed hashes and images are encrypted at rest.

| Easy (difficulty 2) | Hard + dark mode (difficulty 8) |
|:---:|:---:|
| ![Easy CAPTCHA](docs/images/captcha-easy.jpeg) | ![Hard CAPTCHA](docs/images/captcha-hard-dark.jpeg) |

## Features

- **Private by construction** — solutions stored as HMAC-SHA256, images encrypted with
  ChaCha20-Poly1305 under a per-session key. No API returns the answer to a stored session.
- **Measured resistance** — defaults chosen by testing frontier vision models against the actual
  images, not by feel. Ten per-letter deformations. See [tuning](docs/CAPTCHA-TUNING.md).
- **Operable** — layered configuration with provenance, live reload without dropped sessions,
  settings that survive a restart, and automatic rollback of a configuration that breaks startup.
- **Small and self-contained** — static musl binary, in-process SQLite, no external services.
  Multi-arch (amd64 + arm64), published with signed build provenance.
- **Production hardening** — API key auth, per-IP rate limiting (GCRA), session expiry and
  attempt limits, structured logging, optional OpenTelemetry export.

## Quick start

```bash
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  ghcr.io/egeapak/captchapi:latest

curl http://localhost:3000/health
```

For anything persistent, mount a volume — the database lives in `/data`:

```bash
docker volume create captchapi-data
docker run -p 3000:3000 -v captchapi-data:/data \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  ghcr.io/egeapak/captchapi:latest
```

Generate real secrets with `openssl rand -base64 32`. Both are required and must be at least
16 bytes.

### Image tags

| tag | variant | moves |
|---|---|---|
| `2.0.0` | distroless | never |
| `2.0`, `2`, `latest` | distroless | on each patch / minor / stable release |
| `scratch-2.0.0`, `scratch-latest`, … | scratch — 22% smaller to pull | as above |

**Pin a digest in production.** Tags are pointers; a digest is the content hash of the manifest:

```bash
docker buildx imagetools inspect ghcr.io/egeapak/captchapi:2.0.0 \
  --format '{{json .Manifest.Digest}}'

docker pull ghcr.io/egeapak/captchapi@sha256:<digest>
gh attestation verify oci://ghcr.io/egeapak/captchapi:2.0.0 --repo egeapak/captchapi
```

Full tag list, measured image sizes and the three deployment modes are in
[docker/README.md](docker/README.md).

### From source

Requires Rust 1.94+ (see `rust-version` in `Cargo.toml`).

```bash
git clone https://github.com/egeapak/captchapi && cd captchapi
cp .env.example .env      # then set API_KEY_SALT and MASTER_API_KEY
cargo run                 # migrations run automatically on startup
```

## Usage

**1. Create an API key** (one-time, with the master key):

```bash
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer $MASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "Frontend Application"}'
```

Save the returned `api_key` — it is shown once.

**2. Create a session:**

```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"length": 6, "difficulty": 5, "expires_in_seconds": 300}'
```

**3. Show the image** — this endpoint is public, so the browser can load it directly:

```html
<img src="http://localhost:3000/api/v1/sessions/{session_id}/image.jpeg" />
```

**4. Validate the answer:**

```bash
curl -X POST http://localhost:3000/api/v1/sessions/{session_id}/validate \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"solution": "aBc5Xq"}'
```

Validation is **case-sensitive**. The session is deleted on success, and after 3 failed attempts.

## Configuration

Settings resolve through seven layers, highest first:

```
admin API > command line > environment > SQLite store > env file (.env) > config file > default
```

```bash
captchapi config show     # effective values, with the layer each came from, secrets redacted
captchapi config check    # validate and exit — 0 ok, 2 bad, for deployment pipelines
captchapi reload          # re-read every source without a restart
```

Session TTLs, the attempt limit, JPEG quality, the cleanup interval and the log level apply
live. Everything else is captured at startup, and a reload reports the drift rather than
pretending to apply it.

Every setting, its default, whether it reloads, how to persist it across restarts, and how to
recover a stored value that prevents startup: **[docs/CONFIGURATION.md](docs/CONFIGURATION.md)**.

## Documentation

| | |
|---|---|
| [API reference](docs/API.md) | Every endpoint, request/response shapes, error codes, migration notes |
| [Configuration](docs/CONFIGURATION.md) | All settings, precedence, reload, persistence, recovery |
| [CAPTCHA tuning](docs/CAPTCHA-TUNING.md) | How the defaults were measured, and which knob to turn |
| [Docker](docker/README.md) | Tags, image sizes, deployment modes, build from source |
| [Architecture](docs/ARCHITECTURE.md) | Layering, request flow, database schema, security model |
| [Contributing](CONTRIBUTING.md) | Development setup, code quality gates, PR requirements |
| [Security policy](SECURITY.md) | Reporting a vulnerability |
| [Changelog](CHANGELOG.md) | Release history |

### Endpoints at a glance

| Method | Endpoint | Auth |
|---|---|---|
| `GET` | `/health` | none |
| `POST` | `/api/v1/sessions` | API key |
| `GET` | `/api/v1/sessions/{id}` · `/image.jpeg` | none |
| `POST` | `/api/v1/sessions/{id}/validate` | API key |
| `DELETE` | `/api/v1/sessions/{id}` | API key |
| `*` | `/api/v1/api-keys` | master |
| `*` | `/api/v1/admin/*` | master |

See [docs/API.md](docs/API.md) for the full list, including configuration and restart endpoints.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). All changes must pass `cargo fmt`, `cargo clippy`,
`cargo check --all-targets --workspace`, `cargo nextest run --all-features --workspace`, and the
Bruno API suite.

## License

[Apache-2.0](LICENSE). Bundled third-party components are listed in
[THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES) and [NOTICE](NOTICE).
