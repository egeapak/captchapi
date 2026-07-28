# CaptchAPI Build Automation
# Usage: just --list

binary := "captchapi"
build_dir := "docker/bin"
image_name := "captchapi"
image_tag := "latest"

# Default: build amd64 image
default: (docker "amd64")

# Build binary for specific platform (amd64 or arm64)
build platform="amd64":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{platform}}" = "amd64" ]; then
        target="x86_64-unknown-linux-musl"
    elif [ "{{platform}}" = "arm64" ]; then
        target="aarch64-unknown-linux-musl"
    else
        echo "❌ Invalid platform. Use: amd64 or arm64"
        exit 1
    fi
    echo "🔨 Building {{platform}} binary (target: $target)..."
    cross build --release --features otel --target "$target" --bin {{binary}}
    echo "✅ Binary built: target/$target/release/{{binary}}"

# Build Docker image for a platform and variant (distroless or scratch)
docker platform="amd64" variant="distroless": (build platform)
    #!/usr/bin/env bash
    set -euo pipefail
    # `variant` defaults to distroless because that is what the release workflow publishes
    # under the bare tags. A local `captchapi:latest` built on a different base than the one
    # that ships is a trap: it works locally and fails in the registry, or the reverse.
    if [ "{{platform}}" = "amd64" ]; then
        target="x86_64-unknown-linux-musl"
    else
        target="aarch64-unknown-linux-musl"
    fi
    case "{{variant}}" in
        distroless) dockerfile="Dockerfile.static" ;;
        scratch)    dockerfile="Dockerfile.scratch" ;;
        *) echo "❌ Invalid variant. Use: distroless or scratch"; exit 1 ;;
    esac

    echo "📦 Copying {{platform}} binary..."
    mkdir -p {{build_dir}}
    # Staged under both names: Dockerfile.static takes a bare `-musl` binary, while
    # Dockerfile.scratch selects on TARGETARCH like the multi-arch build does.
    cp "target/$target/release/{{binary}}" "{{build_dir}}/{{binary}}-musl"
    cp "target/$target/release/{{binary}}" "{{build_dir}}/{{binary}}-{{platform}}"
    chmod +x "{{build_dir}}/{{binary}}-musl" "{{build_dir}}/{{binary}}-{{platform}}"

    echo "🐳 Building {{variant}} image for linux/{{platform}} ($dockerfile)..."
    cd docker && docker build \
        --platform "linux/{{platform}}" \
        -t {{image_name}}:{{image_tag}} \
        -f "$dockerfile" \
        ..

    echo ""
    echo "✅ Image built: {{image_name}}:{{image_tag}} ({{platform}}, {{variant}})"
    docker images {{image_name}}:{{image_tag}}

# Build images for both architectures (locally)
docker-multiarch:
    @echo "🔨 Building images for both platforms..."
    @echo "Note: Builds separately as multi-arch manifests can't be loaded locally"
    @echo ""
    just docker amd64
    @echo ""
    just docker arm64
    @echo ""
    @echo "✅ Both images built!"
    @echo ""
    @docker images {{image_name}} | head -3

# Build and push a multi-arch image to a registry
push registry tag=image_tag variant="distroless":
    #!/usr/bin/env bash
    set -euo pipefail
    # Releases go through .github/workflows/release.yml, which publishes both variants,
    # smoke-tests them before pushing and attaches provenance. This recipe is for one-off
    # pushes to a private registry.
    case "{{variant}}" in
        distroless) dockerfile="Dockerfile.multiarch" ;;
        scratch)    dockerfile="Dockerfile.scratch"   ;;
        *) echo "❌ Invalid variant. Use: distroless or scratch"; exit 1 ;;
    esac
    echo "🔨 Step 1: Building binaries for both platforms..."
    just build amd64
    just build arm64
    echo ""
    echo "📦 Step 2: Copying binaries..."
    mkdir -p {{build_dir}}
    cp target/x86_64-unknown-linux-musl/release/{{binary}} {{build_dir}}/{{binary}}-amd64
    cp target/aarch64-unknown-linux-musl/release/{{binary}} {{build_dir}}/{{binary}}-arm64
    chmod +x {{build_dir}}/{{binary}}-*
    ls -lh {{build_dir}}/{{binary}}-*
    echo ""
    echo "🐳 Step 3: Building and pushing multi-arch manifest..."
    echo "   Registry: {{registry}}"
    echo "   Tag: {{tag}}"
    echo "   Variant: {{variant}} ($dockerfile)"
    echo "   Platforms: linux/amd64, linux/arm64"
    cd docker && docker buildx build \
        --platform linux/amd64,linux/arm64 \
        -t {{registry}}/{{image_name}}:{{tag}} \
        -f "$dockerfile" \
        --push \
        ..
    echo ""
    echo "✅ Multi-arch image pushed to {{registry}}/{{image_name}}:{{tag}}"

# Run locally
run:
    docker run --rm -p 3000:3000 \
        -e API_KEY_SALT=local-dev-salt-16chars \
        -e MASTER_API_KEY=change-this-to-a-secure-master-key-in-production \
        {{image_name}}:{{image_tag}}

# Run with volume
run-volume:
    docker volume create captchapi-data || true
    docker run --rm -v captchapi-data:/data -p 3000:3000 \
        -e API_KEY_SALT=local-dev-salt-16chars \
        -e MASTER_API_KEY=change-this-to-a-secure-master-key-in-production \
        {{image_name}}:{{image_tag}}

# Clean
clean:
    rm -rf {{build_dir}}

# Clean all
clean-all: clean
    cargo clean
