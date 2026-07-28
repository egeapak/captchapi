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
just build amd64     # Build the musl binary with cross (~5 min first time)
                     # (`just docker amd64` does this and builds the image)
just docker          # Build Docker image (~5 sec)
```

### Run Locally

```bash
# Transient (data lost on restart)
just run

# With persistent volume
just run-volume

# Or manually:
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  captchapi:latest
```

## Using the Published Image

Pre-built multi-platform images are available on GitHub Container Registry:

```bash
# Pull latest
docker pull ghcr.io/egeapak/captchapi:latest

# Pull specific version
docker pull ghcr.io/egeapak/captchapi:2.0.0

# Run
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  ghcr.io/egeapak/captchapi:latest
```

### Published Tags

`.github/workflows/release.yml` publishes on every `v*` tag. **Two variants, both built for
`linux/amd64` and `linux/arm64`, from the same binaries.** A tag of `v1.2.3` produces:

| tag | variant | moves | pull this when |
|-----|---------|-------|----------------|
| `1.2.3` | distroless | never | you want a reproducible, pinned deployment |
| `1.2` | distroless | on each patch | you accept patch updates |
| `1` | distroless | on each minor | you accept minor updates |
| `latest` | distroless | on each stable release | you are evaluating, or always want the newest |
| `scratch-1.2.3` | scratch | never | as above, but you want the smallest possible image |
| `scratch-1.2` | scratch | on each patch | |
| `scratch-1` | scratch | on each minor | |
| `scratch-latest` | scratch | on each stable release | |

**The bare tags are the distroless image and stay that way** — nothing about existing `latest`
deployments changes. The `scratch-` tags are the same binary and the same behaviour on an empty
filesystem: 22% smaller to pull and 40% smaller unpacked, see [Size](#size). Take them if image
size matters more to you than having tzdata, a CA bundle and an `/etc/passwd` in the image.

**A pre-release tag moves none of the moving tags.** `v2.0.0-rc.1` publishes `2.0.0-rc.1` and
`scratch-2.0.0-rc.1` and nothing else, so no one tracking `latest`, `1` or `2.0` is upgraded
onto a release candidate by accident. The same tag also marks the GitHub release as a
pre-release.

Every push is accompanied by a signed build provenance attestation, so a consumer can establish
which workflow run and which commit produced a digest:

```bash
gh attestation verify oci://ghcr.io/egeapak/captchapi:2.0.0 --repo egeapak/captchapi
```

### Pinning by digest

Every tag above is a *pointer*, and even `1.2.3` is only immutable by convention — a digest is
immutable by construction, because it is the SHA-256 of the manifest itself. Production
deployments should pin it:

```bash
# Resolve the digest of a tag
docker buildx imagetools inspect ghcr.io/egeapak/captchapi:2.0.0 \
  --format '{{json .Manifest.Digest}}'

# Deploy that exact image
docker pull ghcr.io/egeapak/captchapi@sha256:<digest>
```

No digest tag needs to be published for this: the registry content-addresses every manifest on
push, so `image@sha256:...` resolves the moment the image exists. Pin the digest of the
*index* — the value the release run reports — not a per-architecture one, so a single reference
keeps working on both `linux/amd64` and `linux/arm64`.

> **The `sha256-<hex>` tags in the Packages UI are not images — do not pull them.** Note the
> shape: `:sha256-abc…`, with a colon and a hyphen, where a digest reference is `@sha256:abc…`.
> They are *tags*, because `:` is not a legal character inside a tag name. Each one holds the
> **provenance attestation** for the image whose digest it names, parked there by
> `actions/attest-build-provenance` with `push-to-registry: true` under the OCI referrers
> fallback convention, for registries without Referrers API support. Verify them with `gh
> attestation verify` above; they are not a way to pin an image.

> **Note for forks.** A package that GitHub Actions creates on ghcr.io starts **private**, and
> nothing in the workflow can change that — `GITHUB_TOKEN` may write packages but may not set
> their visibility. Until someone flips it, every `docker pull` above fails with `unauthorized`
> for anyone who is not a collaborator. Fix it once at
> *Packages → captchapi → Package settings → Danger Zone → Change visibility → Public*.
> This has already been done for `ghcr.io/egeapak/captchapi`.

---

## Image Details

- **Base**: `gcr.io/distroless/static-debian12:nonroot` (bare tags) or `scratch` (`scratch-` tags)
- **Binary**: Fully static musl, no runtime dependencies at all
- **User**: nonroot, UID 65532
- **Security**: no shell, no package manager, no libraries — minimal attack surface

### Size

Measured at commit `b9051ee`, binaries built exactly as the release workflow builds them
(`cross build --release --features otel`):

| variant | platform | pull (compressed) | unpacked |
|---------|----------|-------------------|----------|
| **distroless** (default) | linux/amd64 | **2.77 MB** (2,766,300 B) | **7.33 MB** (7,333,888 B) |
| **distroless** (default) | linux/arm64 | **2.72 MB** (2,716,969 B) | — |
| scratch | linux/amd64 | 2.17 MB (2,167,208 B) | 4.43 MB (4,428,800 B) |
| scratch | linux/arm64 | 2.12 MB (2,117,873 B) | — |

Scratch is 22% smaller to pull and 40% smaller unpacked. Essentially all of the difference is
tzdata, which distroless carries and this service — which stores unix timestamps — never reads.

The static binary itself is 4,228,016 B on amd64 and 3,543,232 B on arm64. Compressed, it is
2,059,727 B of the distroless image's 2,766,300 B — **74% of the pull is the binary**, and 95%
of the scratch image's. The base is not where a remaining win is; any further size work has to
happen in the Rust build.

**Three different numbers exist for "size"; say which one you mean.**

- *Pull* is the sum of the compressed layer blobs the registry actually serves. This is what a
  `docker pull` transfers, and the number worth quoting. Get it from the registry, per platform:
  ```bash
  docker buildx imagetools inspect ghcr.io/egeapak/captchapi:latest --raw   # find the platform digest
  docker buildx imagetools inspect ghcr.io/egeapak/captchapi@<digest> --raw | jq '[.layers[].size] | add'
  ```
- *Unpacked* is the flattened filesystem: `docker export $(docker create <image>) | wc -c`.
- *`docker images`* reports a third, larger figure that includes storage-driver overhead and
  matches neither. It is how the previously advertised "7.42 MB" drifted out of date. Do not
  quote it.

The release workflow measures the first two on every tag and writes them to the run summary, so
this table can be checked against the release rather than trusted.

---

## Data Persistence Configuration

CaptchAPI stores its SQLite database in `/data/captchapi.db`. Choose the persistence mode based on your deployment needs.

### Mode 1: Transient (Testing/Development)

**Use Case:** Testing, CI/CD, temporary instances

**Behavior:**
- Database stored in container's `/data` directory
- Data lost when container stops/restarts
- No volume configuration needed
- Works immediately out of the box

**Command:**
```bash
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  captchapi:latest
```

**Pros:**
- ✅ Zero setup required
- ✅ Fast startup
- ✅ Clean slate every restart

**Cons:**
- ❌ Data lost on restart
- ❌ Not suitable for production

---

### Mode 2: Docker Volume (Production Recommended)

**Use Case:** Production deployments, persistent data

**Behavior:**
- Database stored in Docker-managed volume
- Data persists across container restarts
- Automatic backup/restore with volume
- Volume managed by Docker daemon

**Setup (One-time):**
```bash
# Create the volume
docker volume create captchapi-data

# Volume permissions are handled automatically by the image
# (The /data directory in the image has correct ownership)
```

**Run:**
```bash
docker run -p 3000:3000 \
  -v captchapi-data:/data \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  captchapi:latest
```

**Or with just:**
```bash
just run-volume
```

**Pros:**
- ✅ Data persists across restarts
- ✅ Docker manages volume lifecycle
- ✅ Easy backup: `docker volume export`
- ✅ Portable across hosts
- ✅ No host filesystem dependencies

**Cons:**
- ⚠️ Volume data hidden in Docker area
- ⚠️ Need Docker CLI for direct access

**Backup/Restore:**
```bash
# Backup
docker run --rm -v captchapi-data:/data -v $(pwd):/backup alpine \
  tar czf /backup/captchapi-backup.tar.gz -C /data .

# Restore
docker run --rm -v captchapi-data:/data -v $(pwd):/backup alpine \
  tar xzf /backup/captchapi-backup.tar.gz -C /data
```

---

### Mode 3: Bind Mount (Development/Inspection)

**Use Case:** Local development, database inspection, direct file access

**Behavior:**
- Database stored in host directory
- Direct access to database file
- Useful for debugging and backups

**Setup (One-time):**
```bash
# Create directory and set permissions
mkdir -p ./data
sudo chown -R 65532:65532 ./data  # nonroot user UID

# On macOS (if sudo chown doesn't work):
# Run container as root once to initialize, then run as nonroot
```

**Run:**
```bash
docker run -p 3000:3000 \
  -v $(pwd)/data:/data \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  captchapi:latest
```

**Pros:**
- ✅ Direct file access from host
- ✅ Easy to inspect/backup database
- ✅ Works with SQLite tools (DB Browser)

**Cons:**
- ⚠️ Requires correct host permissions (UID 65532)
- ⚠️ Platform-specific (macOS permission quirks)
- ⚠️ Less portable

---

## Layer Order (Optimized for Caching)

The Dockerfile layers are ordered by change frequency:

1. `/data` directory (never changes) ← Always cached
2. `migrations/` (rarely change) ← Usually cached
3. `captchapi` binary (changes frequently) ← Only layer rebuilt on code changes

This ensures rebuilds only invalidate the binary layer, making builds extremely fast (~5 seconds).

---

## Build Performance

| Scenario | Time | Notes |
|----------|------|-------|
| **First build** | ~5 min | Cross-compiles musl binary |
| **Code change** | ~2 min | Rebuilds binary + Docker (layer cache) |
| **Config change** | ~5 sec | Docker only (all layers cached) |

---

## Clean Up

```bash
just clean      # Remove docker/bin/
just clean-all  # Remove docker/bin/ and target/
```
