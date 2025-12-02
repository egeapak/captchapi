# CaptchAPI Build Automation
# Install just: cargo install just
# Usage: just --list

# Configuration
target := "x86_64-unknown-linux-musl"
binary := "captchapi"
build_dir := "docker/bin"
image_name := "captchapi"
image_tag := "latest"

# Default recipe - builds Docker image
default: docker

# Build musl static binary using cross
build-musl:
    @echo "🔨 Building musl static binary with cross..."
    @echo "   Target: {{target}}"
    cross build --release --target {{target}} --bin {{binary}}
    @echo "✅ Musl binary built: target/{{target}}/release/{{binary}}"

# Copy binary to docker build context
copy-binary: build-musl
    @echo "📦 Copying binary to Docker build context..."
    mkdir -p {{build_dir}}
    cp target/{{target}}/release/{{binary}} {{build_dir}}/{{binary}}-musl
    chmod +x {{build_dir}}/{{binary}}-musl
    @ls -lh {{build_dir}}/{{binary}}-musl
    @echo "✅ Binary ready: {{build_dir}}/{{binary}}-musl"

# Build Docker image with static distroless
docker: copy-binary
    @echo "🐳 Building static Docker image..."
    cd docker && docker build -t {{image_name}}:{{image_tag}} -f Dockerfile.static ..
    @echo ""
    @echo "✅ Docker image built: {{image_name}}:{{image_tag}}"
    @docker images {{image_name}}:{{image_tag}}

# Run the image locally for testing
run:
    @echo "🚀 Running {{image_name}}:{{image_tag}} on port 3000..."
    docker run --rm -p 3000:3000 {{image_name}}:{{image_tag}}

# Run with persistent volume
run-volume:
    @echo "🚀 Running with persistent volume..."
    docker volume create captchapi-data || true
    docker run --rm -v captchapi-data:/data -p 3000:3000 {{image_name}}:{{image_tag}}

# Clean build artifacts
clean:
    @echo "🧹 Cleaning build artifacts..."
    rm -rf {{build_dir}}
    @echo "✅ Cleaned {{build_dir}}"

# Clean everything including cargo artifacts
clean-all: clean
    cargo clean
    @echo "✅ Cleaned cargo artifacts"
