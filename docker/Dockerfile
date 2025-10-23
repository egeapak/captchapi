# ============================================================
# Stage 1: Builder
# ============================================================
FROM rust:1.90-slim AS builder

# Install build dependencies
RUN apt-get update && \
    apt-get install -y pkg-config libsqlite3-dev && \
    rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /build

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./

# Create dummy main.rs to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Copy actual source code and migrations
COPY src ./src
COPY migrations ./migrations

# Build the actual application
RUN cargo build --release --bin captchapi

# Create /data directory with correct ownership for nonroot user (UID 65532)
# This enables the transient (no-volume) use case to work out of the box
RUN mkdir -p /data && chown 65532:65532 /data

# ============================================================
# Stage 2: Runtime (Distroless)
# ============================================================
FROM gcr.io/distroless/cc-debian12:nonroot

# Set working directory
WORKDIR /app

# Copy binary, migrations, and /data directory with nonroot ownership
# The --chown flag ensures proper permissions for the nonroot user (UID:GID 65532:65532)
COPY --from=builder --chown=nonroot:nonroot /build/target/release/captchapi /app/captchapi
COPY --from=builder --chown=nonroot:nonroot /build/migrations /app/migrations
COPY --from=builder --chown=nonroot:nonroot /data /data

# Expose port
EXPOSE 3000

# This image runs as user 'nonroot' (UID 65532, GID 65532) for security
#
# THREE PERSISTENCE OPTIONS:
# ===========================
# 1. NO VOLUME (Transient) - Works out of the box, data lost on container restart
#    docker run -p 3000:3000 captchapi
#
# 2. DOCKER VOLUME (Managed by Docker) - Requires one-time permission setup
#    docker volume create captchapi-data
#    docker run --rm -v captchapi-data:/data alpine chown -R 65532:65532 /data
#    docker run -p 3000:3000 -v captchapi-data:/data captchapi
#
# 3. BIND MOUNT (Host directory) - Host directory must be writable by UID 65532
#    mkdir -p ./data && sudo chown -R 65532:65532 ./data
#    docker run -p 3000:3000 -v $(pwd)/data:/data captchapi
#
# See docker-compose.yml for orchestrated examples

# Entrypoint
ENTRYPOINT ["/app/captchapi"]
