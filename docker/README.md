# Docker Build Guide

## Quick Start

### Prerequisites
- Install [cross](https://github.com/cross-rs/cross): `cargo install cross`
- Install [just](https://github.com/casey/just): `cargo install just`

### Build Static Image

```bash
# Build everything (default)
just

# Or step by step:
just build-musl      # Build musl binary with cross
just copy-binary     # Copy to docker/bin/
just docker          # Build Docker image
```

### Run

```bash
# Transient (data lost on restart)
just run

# With persistent volume
just run-volume

# Or manually:
docker run -p 3000:3000 captchapi:latest
```

## Image Details

- **Base**: `gcr.io/distroless/static-debian12:nonroot`
- **Size**: ~7.5 MB
- **Binary**: Fully static musl (no dependencies)
- **User**: nonroot (UID 65532)
- **Security**: Maximum (no shell, no libraries, minimal attack surface)

## Layer Order (Optimized for Caching)

The Dockerfile layers are ordered by change frequency:

1. `/data` directory (never changes)
2. `migrations/` (rarely change)
3. `captchapi` binary (changes frequently)

This ensures rebuilds only invalidate the binary layer.

## Clean Up

```bash
just clean      # Remove docker/bin/
just clean-all  # Remove docker/bin/ and target/
```
