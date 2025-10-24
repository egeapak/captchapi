#!/usr/bin/env bash

# Pre-release script for captchapi
# This script prepares a new release by:
# 1. Updating Cargo.toml version
# 2. Generating CHANGELOG.md using git-cliff
# 3. Creating a release commit
# 4. Tagging the release
# 5. Pushing to master

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
error() {
    echo -e "${RED}❌ Error: $1${NC}" >&2
    exit 1
}

success() {
    echo -e "${GREEN}✅ $1${NC}"
}

info() {
    echo -e "${BLUE}ℹ️  $1${NC}"
}

warning() {
    echo -e "${YELLOW}⚠️  $1${NC}"
}

# Check if git-cliff is installed
check_git_cliff() {
    if ! command -v git-cliff &> /dev/null; then
        error "git-cliff is not installed. Install it with: cargo install git-cliff"
    fi
}

# Check if we're on master branch
check_branch() {
    local branch=$(git rev-parse --abbrev-ref HEAD)
    if [[ "$branch" != "master" ]]; then
        error "You must be on master branch to create a release. Current branch: $branch"
    fi
}

# Check for uncommitted changes
check_uncommitted_changes() {
    if [[ -n $(git status --porcelain) ]]; then
        warning "You have uncommitted changes:"
        git status --short
        echo
        read -p "Do you want to continue anyway? (y/N) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            error "Aborted by user"
        fi
    fi
}

# Validate version format (semver)
validate_version() {
    local version=$1
    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        error "Invalid version format: $version. Expected format: X.Y.Z (e.g., 1.0.0)"
    fi
}

# Get current version from Cargo.toml
get_current_version() {
    grep '^version = ' Cargo.toml | head -1 | cut -d'"' -f2
}

# Update version in Cargo.toml
update_cargo_version() {
    local new_version=$1
    local current_version=$(get_current_version)

    info "Updating version in Cargo.toml: $current_version → $new_version"

    # Use sed to update version (compatible with both macOS and Linux)
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s/^version = \"$current_version\"/version = \"$new_version\"/" Cargo.toml
    else
        sed -i "s/^version = \"$current_version\"/version = \"$new_version\"/" Cargo.toml
    fi

    success "Updated Cargo.toml version to $new_version"
}

# Generate changelog using git-cliff
generate_changelog() {
    local version=$1

    info "Generating CHANGELOG.md for version $version..."

    if git-cliff --tag "v$version" -o CHANGELOG.md; then
        success "Generated CHANGELOG.md"
    else
        error "Failed to generate CHANGELOG.md"
    fi
}

# Create release commit
create_release_commit() {
    local version=$1

    info "Creating release commit..."

    git add Cargo.toml CHANGELOG.md Cargo.lock 2>/dev/null || true

    if git commit -m "chore(release): prepare for v$version"; then
        success "Created release commit"
    else
        error "Failed to create release commit"
    fi
}

# Create git tag
create_tag() {
    local version=$1

    info "Creating tag v$version..."

    if git tag -a "v$version" -m "Release v$version"; then
        success "Created tag v$version"
    else
        error "Failed to create tag"
    fi
}

# Push to master
push_to_master() {
    local version=$1

    info "Ready to push to master..."
    echo
    echo "This will push:"
    echo "  - Release commit (chore(release): prepare for v$version)"
    echo "  - Tag v$version"
    echo
    read -p "Do you want to push to master now? (y/N) " -n 1 -r
    echo

    if [[ $REPLY =~ ^[Yy]$ ]]; then
        info "Pushing to master..."

        if git push origin master && git push origin "v$version"; then
            success "Pushed to master!"
            echo
            success "Release v$version is ready! 🎉"
            echo
            info "The release workflow will now:"
            echo "  1. Validate version match"
            echo "  2. Build multi-arch Docker images"
            echo "  3. Push to ghcr.io"
            echo "  4. Create GitHub release"
            echo
            info "Monitor the workflow at: https://github.com/egeapak/captchapi/actions"
        else
            error "Failed to push to master. You may need to push manually."
        fi
    else
        warning "Skipped push to master."
        echo
        info "To push manually, run:"
        echo "  git push origin master"
        echo "  git push origin v$version"
    fi
}

# Show usage
usage() {
    cat << EOF
Usage: $0 <version>

Prepare a new release by updating version, generating changelog, and creating a release commit.

Arguments:
  version    Version number in semver format (e.g., 1.0.0)

Examples:
  $0 1.0.0
  $0 0.2.1

Requirements:
  - git-cliff must be installed (cargo install git-cliff)
  - Must be on master branch
  - Clean working directory recommended

What this script does:
  1. Updates version in Cargo.toml
  2. Generates CHANGELOG.md using git-cliff
  3. Creates release commit
  4. Creates git tag
  5. Pushes to master (with confirmation)

After pushing, the GitHub Actions release workflow will automatically:
  - Build Docker images for linux/amd64 and linux/arm64
  - Push to GitHub Container Registry
  - Create a GitHub release
EOF
}

# Main script
main() {
    # Check arguments
    if [[ $# -ne 1 ]]; then
        usage
        exit 1
    fi

    local version=$1

    # Show banner
    echo
    echo "=================================================="
    echo "  CaptchAPI Release Preparation Script"
    echo "=================================================="
    echo

    # Run checks
    info "Running pre-flight checks..."
    check_git_cliff
    validate_version "$version"
    check_branch
    check_uncommitted_changes

    local current_version=$(get_current_version)

    echo
    info "Current version: $current_version"
    info "New version: $version"
    echo

    # Confirm with user
    read -p "Continue with release preparation? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        error "Aborted by user"
    fi

    echo
    info "Starting release preparation..."
    echo

    # Update version
    update_cargo_version "$version"

    # Generate changelog
    generate_changelog "$version"

    # Create commit
    create_release_commit "$version"

    # Create tag
    create_tag "$version"

    echo
    success "Release preparation complete!"
    echo

    # Push to master
    push_to_master "$version"
}

# Run main function
main "$@"
