# Contributing to CaptchAPI

Thank you for your interest in contributing to CaptchAPI! This guide will help you get started.

## Development Setup

1. **Prerequisites**: Rust 1.85+ and Cargo
2. Clone the repository: `git clone https://github.com/egeapak/captchapi && cd captchapi`
3. Copy environment template: `cp .env.example .env`
4. Update `.env` with secure values (`API_KEY_SALT` and `MASTER_API_KEY` must be at least 16 characters)
5. Build: `cargo build`
6. Run: `cargo run`

## Architecture

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for project structure, layered architecture, database schema, and key design decisions.

## Code Quality Standards

After **every** code change, run these steps **in order**:

```bash
cargo fmt          # Format code
cargo clippy       # Run linter
cargo check        # Check compilation
cargo nextest run  # Run all tests
```

For API-level testing (requires running server):
```bash
./.bruno/Tests/Scripts/test-bruno-full.sh
```

All steps must pass before submitting a PR.

## Pull Request Requirements

- All tests passing (Rust + API tests)
- No clippy warnings
- Code formatted with `cargo fmt`
- Documentation updated if applicable
- Clear description of changes and motivation
- Reference any related issues

## Testing Requirements

Every new endpoint must include:
- Rust integration tests (success and failure cases)
- Bruno test scenarios in `.bruno/Tests/`
- Bruno core endpoint in `.bruno/` root directory

## Reporting Issues

Please use [GitHub Issues](https://github.com/egeapak/captchapi/issues) to report bugs or request features.
