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
just build-musl      # Build musl binary with cross (~5 min first time)
just copy-binary     # Copy to docker/bin/
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
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
  captchapi:latest
```

## Using the Published Image

Pre-built multi-platform images are available on GitHub Container Registry:

```bash
# Pull latest
docker pull ghcr.io/egeapak/captchapi:latest

# Pull specific version
docker pull ghcr.io/egeapak/captchapi:1.0.0

# Run
docker run -p 3000:3000 \
  -e API_KEY_SALT=your-salt-minimum-16chars \
  -e MASTER_API_KEY=your-key-minimum-16chars \
  ghcr.io/egeapak/captchapi:latest
```

---

## Image Details

- **Base**: `gcr.io/distroless/static-debian12:nonroot`
- **Size**: 7.42 MB (84.5% smaller than original)
- **Binary**: Fully static musl (no dependencies)
- **User**: nonroot (UID 65532)
- **Security**: Maximum (no shell, no libraries, minimal attack surface)
- **Efficiency**: 100% (only 613 bytes wasted)

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
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
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
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
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
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-key \
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

### Kubernetes Deployment

For Kubernetes, use PersistentVolumeClaims:

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: captchapi-data
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 1Gi
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: captchapi
spec:
  replicas: 1
  selector:
    matchLabels:
      app: captchapi
  template:
    metadata:
      labels:
        app: captchapi
    spec:
      containers:
      - name: captchapi
        image: captchapi:latest
        ports:
        - containerPort: 3000
        env:
        - name: API_KEY_SALT
          valueFrom:
            secretKeyRef:
              name: captchapi-secrets
              key: api-key-salt
        - name: MASTER_API_KEY
          valueFrom:
            secretKeyRef:
              name: captchapi-secrets
              key: master-api-key
        volumeMounts:
        - name: data
          mountPath: /data
        securityContext:
          runAsUser: 65532
          runAsGroup: 65532
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true  # Except /data
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: captchapi-data
```

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
