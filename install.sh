#!/usr/bin/env bash
set -euo pipefail

REPO="dehs05606-dotcom/rsclitest"
VERSION="v1.0.0"
BIN="aia-agent"
INSTALL_DIR="/usr/local/bin"

if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
    echo "Downloading $BIN $VERSION..."
    curl -sL "https://github.com/$REPO/releases/download/$VERSION/$BIN" -o /tmp/$BIN
    chmod +x /tmp/$BIN
    sudo mv /tmp/$BIN "$INSTALL_DIR/$BIN"
    echo "Installed to $INSTALL_DIR/$BIN"
    echo ""
    echo "Run: aia-agent chat"
else
    echo "Unsupported platform. Build from source:"
    echo "  git clone https://github.com/$REPO.git"
    echo "  cd rsclitest && cargo build --release && ./target/release/aia-agent"
fi
