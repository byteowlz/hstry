# Packaging Scripts

This directory contains scripts for setting up and managing releases for hstry.

## Scripts

### `setup-all.sh`
Complete setup script that checks all release infrastructure prerequisites.

```bash
./packaging/setup-all.sh
```

What it does:
- Checks if git and gh CLI are installed
- Verifies Homebrew tap configuration
- Checks AUR SSH key setup
- Displays required secrets and setup steps

### `setup-homebrew-tap.sh` (Reference)
Script to set up the Homebrew tap repository (for reference only, tap already exists at `byteowlz/homebrew-tap`).

### `setup-aur.sh`
Script to set up the AUR package.

```bash
./packaging/setup-aur.sh
```

What it does:
- Clones or initializes AUR package repository
- Generates initial PKGBUILD
- Generates .SRCINFO
- Provides setup instructions

## Quick Start

1. Run the setup check:
   ```bash
   ./packaging/setup-all.sh
   ```

2. Set up Homebrew token:
   - Generate PAT at https://github.com/settings/tokens
   - Add `TAP_GITHUB_TOKEN` secret to hstry repo

3. Set up AUR:
   - Generate SSH key: `ssh-keygen -t ed25519 -f ~/.ssh/aur`
   - Add public key to AUR account
   - Run: `./packaging/setup-aur.sh`
   - Add `AUR_SSH_PRIVATE_KEY` and `AUR_EMAIL` secrets to hstry repo

## Doing a Release

Manual release:

```bash
# Update version in Cargo.toml workspace
vim Cargo.toml

# Update CHANGELOG.md
vim CHANGELOG.md

# Commit and tag
git add Cargo.toml CHANGELOG.md
git commit -m "chore: bump version to 0.4.4"
git tag v0.4.4
git push origin main
git push origin v0.4.4
```

## Release Artifacts

Each release includes:

### GitHub Releases
- Binaries for: Linux x86_64/ARM64, macOS Intel/Apple Silicon
- SHA256 checksums file
- Auto-generated release notes

### Homebrew
- Formula in `byteowlz/homebrew-tap/Formula/hstry.rb`
- Automatically updated via GitHub Actions

### AUR
- PKGBUILD at https://aur.archlinux.org/packages/hstry
- Automatically updated via GitHub Actions

## Binaries

The release packages the following binaries when available:
- `hstry` - CLI tool (always included)
- `hstry-tui` - Terminal UI (optional)
- `hstry-mcp` - MCP server (optional)

## Support

For issues or questions, see [docs/RELEASE.md](../docs/RELEASE.md) or open an issue on GitHub.
