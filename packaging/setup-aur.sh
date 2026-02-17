#!/usr/bin/env bash
set -euo pipefail

# Script to set up AUR package for hstry
# Usage: ./setup-aur.sh

echo "Setting up AUR package for hstry..."
echo ""

# Check if git config is set
if ! git config user.name >/dev/null || ! git config user.email >/dev/null; then
    echo "Error: Git user.name and user.email must be configured"
    echo "Run:"
    echo "  git config --global user.name 'Your Name'"
    echo "  git config --global user.email 'your@email.com'"
    exit 1
fi

# Clone the AUR package if it doesn't exist
if [ ! -d "aur-hstry" ]; then
    echo "Cloning hstry from AUR (this may fail if package doesn't exist yet)..."
    if git clone ssh://aur@aur.archlinux.org/hstry.git aur-hstry 2>/dev/null; then
        echo "Package already exists in AUR"
    else
        echo "Package doesn't exist in AUR yet, creating local directory..."
        mkdir -p aur-hstry
        cd aur-hstry
        git init
        git remote add origin ssh://aur@aur.archlinux.org/hstry.git
        cd ..
    fi
else
    echo "AUR directory already exists"
fi

# Generate initial PKGBUILD
cat > aur-hstry/PKGBUILD << 'EOF'
# Maintainer: byteowlz <dev@byteowlz.com>
pkgname=hstry
pkgver=0.4.3
pkgrel=1
pkgdesc="Universal AI chat history database with full-text search"
arch=('x86_64' 'aarch64')
url="https://github.com/byteowlz/hstry"
license=('MIT')
depends=('gcc-libs' 'sqlite')
optdepends=('bash: for shell completions' 'zsh: for shell completions')
source_x86_64=("$pkgname-$pkgver.tar.gz::https://github.com/byteowlz/hstry/releases/download/v$pkgver/hstry-v$pkgver-x86_64-unknown-linux-gnu.tar.gz")
source_aarch64=("$pkgname-$pkgver.tar.gz::https://github.com/byteowlz/hstry/releases/download/v$pkgver/hstry-v$pkgver-aarch64-unknown-linux-gnu.tar.gz")
sha256sums_x86_64=('TBD')
sha256sums_aarch64=('TBD')

package() {
    install -Dm755 hstry "$pkgdir/usr/bin/hstry"
    if [ -f "hstry-tui" ]; then
        install -Dm755 hstry-tui "$pkgdir/usr/bin/hstry-tui"
    fi
    if [ -f "hstry-mcp" ]; then
        install -Dm755 hstry-mcp "$pkgdir/usr/bin/hstry-mcp"
    fi
}
EOF

# Generate .SRCINFO
cat > aur-hstry/.SRCINFO << 'EOF'
pkgbase = hstry
	pkgdesc = Universal AI chat history database with full-text search
	pkgver = 0.4.3
	pkgrel = 1
	url = https://github.com/byteowlz/hstry
	arch = x86_64
	arch = aarch64
	license = MIT
	depends = gcc-libs
	depends = sqlite
	optdepends = bash: for shell completions
	optdepends = zsh: for shell completions
	source_x86_64 = hstry-0.4.3.tar.gz::https://github.com/byteowlz/hstry/releases/download/v0.4.3/hstry-v0.4.3-x86_64-unknown-linux-gnu.tar.gz
	sha256sums_x86_64 = TBD
	source_aarch64 = hstry-0.4.3.tar.gz::https://github.com/byteowlz/hstry/releases/download/v0.4.3/hstry-v0.4.3-aarch64-unknown-linux-gnu.tar.gz
	sha256sums_aarch64 = TBD

pkgname = hstry
EOF

echo ""
echo "AUR package setup complete!"
echo ""
echo "Directory: aur-hstry/"
echo ""
echo "Next steps:"
echo "1. Generate SSH key for AUR (if you haven't already):"
echo "   ssh-keygen -t ed25519 -f ~/.ssh/aur -C 'AUR SSH key'"
echo ""
echo "2. Add the SSH key to your AUR account:"
echo "   - Go to https://aur.archlinux.org/account/<username>/edit"
echo "   - Paste contents of ~/.ssh/aur.pub"
echo ""
echo "3. Test SSH connection:"
echo "   ssh -i ~/.ssh/aur aur@aur.archlinux.org"
echo ""
echo "4. Add AUR_SSH_PRIVATE_KEY and AUR_EMAIL secrets to your hstry repository:"
echo "   - Go to https://github.com/byteowlz/hstry/settings/secrets/actions"
echo "   - Add AUR_SSH_PRIVATE_KEY with the content of ~/.ssh/aur"
echo "   - Add AUR_EMAIL with your email address"
echo ""
echo "5. Push the initial package (only if it doesn't exist in AUR yet):"
echo "   cd aur-hstry"
echo "   git add ."
echo "   git commit -m 'Initial import of hstry'"
echo "   git push -u origin main"
echo ""
echo "Note: The release workflow will automatically update this package on future releases."
