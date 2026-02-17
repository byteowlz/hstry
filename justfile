# hstry justfile (Rust workspace)

# Default recipe - show help
default:
    @just --list

# === Development ===

# Install all binaries
install:
    cargo install --path .

# Install all binaries from workspace
install-all:
    @for crate in $(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.targets[] | .kind[] == "bin") | .manifest_path | split("/") | .[-2]'); do \
        echo "Installing $crate..."; \
        cargo install --path crates/$crate; \
    done

# Install a specific crate
install-crate CRATE:
    cargo install --path crates/{{CRATE}}

# Uninstall all binaries
uninstall:
    @for crate in $(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.targets[] | .kind[] == "bin") | .name'); do \
        echo "Uninstalling $crate..."; \
        cargo uninstall $crate 2>/dev/null || true; \
    done

# === Building ===

# Debug build (all crates)
build:
    cargo build --workspace

# Release build (all crates)
build-release:
    cargo build --workspace --release

# Build a specific crate
build-crate CRATE:
    cargo build -p {{CRATE}}

# Build with all features
build-all:
    cargo build --workspace --all-features

# Fast compile check
check:
    cargo check --workspace

# Check a specific crate
check-crate CRATE:
    cargo check -p {{CRATE}}

# Clean build artifacts
clean:
    cargo clean

# === Testing ===

# Run all tests
test:
    cargo test --workspace

# Run tests for a specific crate
test-crate CRATE:
    cargo test -p {{CRATE}}

# Run tests with all features
test-all:
    cargo test --workspace --all-features

# Run tests verbosely
test-v:
    cargo test --workspace -- --nocapture

# Run a specific test
test-one TEST:
    cargo test --workspace {{TEST}}

# === Code Quality ===

# Format all code
fmt:
    cargo fmt --all

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Run clippy on all crates
clippy:
    cargo clippy --workspace -- -D warnings

# Alias for clippy
lint: clippy

# Clippy on a specific crate
clippy-crate CRATE:
    cargo clippy -p {{CRATE}} -- -D warnings

# Auto-fix clippy warnings
fix:
    cargo clippy --workspace --fix --allow-dirty

# Run all checks
check-all: fmt-check clippy test

# === Documentation ===

# Generate docs for all crates
docs:
    cargo doc --workspace --no-deps

# Generate and open docs
docs-open:
    cargo doc --workspace --no-deps --open

# Docs for a specific crate
docs-crate CRATE:
    cargo doc -p {{CRATE}} --no-deps --open

# === Dependencies ===

# Update all dependencies
update:
    cargo update

# Check for outdated dependencies
outdated:
    cargo outdated --workspace

# === Workspace Info ===

# List all crates in workspace
list:
    @cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | "\(.name) (\(.version))"'

# List binary crates
list-bins:
    @cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.targets[] | .kind[] == "bin") | .name'

# List library crates
list-libs:
    @cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.targets[] | .kind[] == "lib") | .name'

# === Release ===

# Release build and show binary sizes
release: build-release
    @echo "Binary sizes:"
    @find target/release -maxdepth 1 -type f -perm +111 ! -name "*.d" -exec ls -lh {} \; 2>/dev/null || true

# Release: bump version, commit, tag, and push
release-bump version:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="{{version}}"
    if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "Error: Version must be in format X.Y.Z"
        exit 1
    fi
    echo "Bumping version to $VERSION"
    # Update workspace version
    sed -i "s/^version = .*/version = \"$VERSION\"/" Cargo.toml
    git add Cargo.toml
    git commit -m "chore: bump version to $VERSION"
    git tag "v$VERSION"
    git push origin main
    git push origin "v$VERSION"
    echo "Release v$VERSION pushed! Workflow will start automatically."

# Check release readiness
release-check:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Checking release readiness..."
    cargo test --workspace --quiet
    cargo clippy --workspace --quiet -- -D warnings
    cargo fmt --all -- --check
    echo "All checks passed!"

# Tag and push a release
release-tag VERSION:
    git tag v{{VERSION}}
    git push --tags

# Set up GitHub secrets for automated releases (requires byt)
setup-secrets:
    byt secrets setup hstry

# Update CHANGELOG for release
changelog version:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="{{version}}"
    DATE=$(date +%Y-%m-%d)
    echo "## [$VERSION] - $DATE" > /tmp/changelog_new.md
    echo "" >> /tmp/changelog_new.md
    echo "### Added" >> /tmp/changelog_new.md
    echo "" >> /tmp/changelog_new.md
    echo "### Changed" >> /tmp/changelog_new.md
    echo "" >> /tmp/changelog_new.md
    echo "### Fixed" >> /tmp/changelog_new.md
    cat CHANGELOG.md >> /tmp/changelog_new.md
    mv /tmp/changelog_new.md CHANGELOG.md
    echo "CHANGELOG.md updated for $VERSION"

# === Adapters ===

# Copy latest adapters to config directory (overwrites existing)
update-adapters:
    @mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/hstry/adapters"
    @rm -rf "${XDG_CONFIG_HOME:-$HOME/.config}/hstry/adapters/"*
    @cp -r adapters/* "${XDG_CONFIG_HOME:-$HOME/.config}/hstry/adapters/"
    @echo "Adapters updated in ${XDG_CONFIG_HOME:-$HOME/.config}/hstry/adapters"
