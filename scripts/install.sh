#!/bin/bash
# Symbiont One-Liner Installer
# Usage: curl -fsSL https://get.symbi.sh | bash

set -e

VERSION="latest"
INSTALL_DIR="${SYMBI_INSTALL_DIR:-$HOME/.symbi}"
BIN_DIR="$INSTALL_DIR/bin"
GITHUB_REPO="thirdkeyai/symbiont"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}╔════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║   Symbiont Installation Script        ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════╝${NC}"
echo

# Detect OS and architecture
detect_platform() {
    local os=$(uname -s | tr '[:upper:]' '[:lower:]')
    local arch=$(uname -m)

    case "$os" in
        linux*)
            OS="linux"
            ;;
        darwin*)
            OS="darwin"
            ;;
        mingw*|msys*|cygwin*)
            OS="windows"
            ;;
        *)
            echo -e "${RED}✗ Unsupported OS: $os${NC}"
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64|amd64)
            ARCH="x86_64"
            ;;
        aarch64|arm64)
            ARCH="aarch64"
            ;;
        *)
            echo -e "${RED}✗ Unsupported architecture: $arch${NC}"
            exit 1
            ;;
    esac

    PLATFORM="${OS}-${ARCH}"

    # Map to Rust target triple
    case "${OS}-${ARCH}" in
        linux-x86_64)   RUST_TARGET="x86_64-unknown-linux-gnu" ;;
        linux-aarch64)  RUST_TARGET="aarch64-unknown-linux-gnu" ;;
        darwin-aarch64) RUST_TARGET="aarch64-apple-darwin" ;;
        darwin-x86_64)
            echo -e "${YELLOW}  No pre-built binary for Intel macOS.${NC}"
            echo -e "${YELLOW}  Install from source: cargo install symbi${NC}"
            echo -e "${YELLOW}  Or use Homebrew: brew tap thirdkeyai/tap && brew install symbi${NC}"
            exit 1
            ;;
        *)
            echo -e "${RED}✗ No pre-built binary for ${OS}-${ARCH}${NC}"
            echo -e "${YELLOW}  Install from source: cargo install symbi${NC}"
            exit 1
            ;;
    esac

    echo -e "• Detected platform: ${GREEN}${PLATFORM}${NC}"
}

# Check prerequisites
check_prerequisites() {
    echo "• Checking prerequisites..."

    # Check for curl or wget
    if ! command -v curl &> /dev/null && ! command -v wget &> /dev/null; then
        echo -e "${RED}✗ Neither curl nor wget found. Please install one of them.${NC}"
        exit 1
    fi

    # Check for tar
    if ! command -v tar &> /dev/null; then
        echo -e "${RED}✗ tar not found. Please install tar.${NC}"
        exit 1
    fi

    echo -e "${GREEN}✓ Prerequisites met${NC}"
}

# Download and install binary
install_binary() {
    echo "• Downloading Symbiont ${VERSION}..."

    mkdir -p "$BIN_DIR"

    # Determine download URLs
    local base_url
    if [ "$VERSION" = "latest" ]; then
        base_url="https://github.com/${GITHUB_REPO}/releases/latest/download"
    else
        base_url="https://github.com/${GITHUB_REPO}/releases/download/${VERSION}"
    fi

    local archive="symbi-${VERSION}-${RUST_TARGET}.tar.gz"
    local download_url="${base_url}/${archive}"
    local checksums_url="${base_url}/checksums.txt"

    # Download binary archive
    local temp_dir=$(mktemp -d)
    local temp_file="${temp_dir}/${archive}"
    local temp_checksums="${temp_dir}/checksums.txt"

    echo -e "${YELLOW}  Note: Pre-built binaries are tested but considered less reliable than${NC}"
    echo -e "${YELLOW}  cargo install or Docker. See https://docs.symbi.sh for alternatives.${NC}"

    if command -v curl &> /dev/null; then
        curl -fsSL "$download_url" -o "$temp_file" || {
            echo -e "${RED}✗ Failed to download Symbiont${NC}"
            echo -e "${YELLOW}  URL: ${download_url}${NC}"
            echo -e "${YELLOW}  Binary releases may not be available for this version/platform.${NC}"
            echo -e "${YELLOW}  Install from source: cargo install symbi${NC}"
            rm -rf "$temp_dir"
            exit 1
        }
        curl -fsSL "$checksums_url" -o "$temp_checksums" 2>/dev/null
    else
        wget -q "$download_url" -O "$temp_file" || {
            echo -e "${RED}✗ Failed to download Symbiont${NC}"
            rm -rf "$temp_dir"
            exit 1
        }
        wget -q "$checksums_url" -O "$temp_checksums" 2>/dev/null
    fi

    # Verify checksum if available
    if [ -f "$temp_checksums" ] && command -v sha256sum &> /dev/null; then
        echo "• Verifying checksum..."
        cd "$temp_dir"
        grep "$archive" checksums.txt | sha256sum --check --quiet && {
            echo -e "${GREEN}✓ Checksum verified${NC}"
        } || {
            echo -e "${RED}✗ Checksum verification failed!${NC}"
            rm -rf "$temp_dir"
            exit 1
        }
        cd - > /dev/null
    fi

    # Extract binary
    tar -xzf "$temp_file" -C "$BIN_DIR"
    rm -rf "$temp_dir"

    chmod +x "$BIN_DIR/symbi"
    echo -e "${GREEN}✓ Binary installed to ${BIN_DIR}/symbi${NC}"
}

# Create quick start config
create_quick_config() {
    local config_file="symbi.quick.toml"

    if [ ! -f "$config_file" ]; then
        echo "• Creating quick start configuration..."

        # Generate a secure dev token
        local dev_token=$(cat /dev/urandom | tr -dc 'a-zA-Z0-9' | fold -w 32 | head -n 1 2>/dev/null || echo "dev")

        cat > "$config_file" << EOF
# Symbiont Quick Start Configuration
# Generated by install script

[runtime]
mode = "dev"
hot_reload = true

[http]
enabled = true
port = 8081
dev_token = "$dev_token"

[storage]
type = "sqlite"
path = "./symbi.db"

[logging]
level = "info"
format = "pretty"
EOF

        echo -e "${GREEN}✓ Created ${config_file}${NC}"
        echo -e "${YELLOW}⚠️  Your dev token: ${dev_token}${NC}"
    fi
}

# Update PATH
update_path() {
    echo "• Updating PATH..."

    local shell_rc=""
    if [ -n "$BASH_VERSION" ]; then
        shell_rc="$HOME/.bashrc"
    elif [ -n "$ZSH_VERSION" ]; then
        shell_rc="$HOME/.zshrc"
    else
        shell_rc="$HOME/.profile"
    fi

    # Check if PATH already contains symbi
    if ! echo "$PATH" | grep -q "$BIN_DIR"; then
        echo "export PATH=\"\$PATH:$BIN_DIR\"" >> "$shell_rc"
        echo -e "${GREEN}✓ Added ${BIN_DIR} to PATH in ${shell_rc}${NC}"
        echo -e "${YELLOW}  Run: source ${shell_rc}${NC}"
    else
        echo -e "${GREEN}✓ PATH already contains ${BIN_DIR}${NC}"
    fi

    # Add to current PATH
    export PATH="$PATH:$BIN_DIR"
}

# Run post-install checks
post_install() {
    echo
    echo -e "${GREEN}✅ Installation complete!${NC}"
    echo
    echo "📝 Next steps:"
    echo "  1. Reload your shell or run: source ~/.bashrc (or ~/.zshrc)"
    echo "  2. Verify installation: symbi --version"
    echo "  3. Check system health: symbi doctor"
    echo "  4. Create a project: symbi new webhook-min my-webhook"
    echo "  5. Start the runtime: symbi up"
    echo
    echo "📚 Documentation: https://docs.symbi.sh"
    echo "🐛 Issues: https://github.com/${GITHUB_REPO}/issues"
    echo
}

# Main installation flow
main() {
    detect_platform
    check_prerequisites
    install_binary
    update_path
    create_quick_config
    post_install
}

# Run installation
main
