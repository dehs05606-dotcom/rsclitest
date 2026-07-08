#!/usr/bin/env bash
set -euo pipefail

REPO="dehs05606-dotcom/rustcli"
BIN="rustcli"

detect_arch() {
    local arch
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) echo "unsupported" ;;
    esac
}

detect_os() {
    local os
    os="$(uname -s)"
    case "$os" in
        Linux) echo "linux" ;;
        Darwin) echo "darwin" ;;
        *) echo "unsupported" ;;
    esac
}

get_latest_version() {
    curl -sSfL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | grep '"tag_name"' \
        | cut -d'"' -f4 \
        || echo "v0.4.0"
}

main() {
    local os arch version url install_dir
    os="$(detect_os)"
    arch="$(detect_arch)"

    if [[ "$os" == "unsupported" || "$arch" == "unsupported" ]]; then
        echo "Unsupported platform: $(uname -s) $(uname -m)"
        echo "Build from source:"
        echo "  git clone https://github.com/$REPO.git"
        echo "  cd rustcli && cargo build --release"
        exit 1
    fi

    version="$(get_latest_version)"
    url="https://github.com/$REPO/releases/download/$version/$BIN"
    install_dir="/usr/local/bin"

    echo "Downloading $BIN $version ($os/$arch)..."
    local tmpfile="/tmp/${BIN}-download"
    if command -v sudo &>/dev/null; then
        curl -sSfL "$url" -o "$tmpfile" || {
            echo "Download failed (URL: $url)"; exit 1
        }
        chmod +x "$tmpfile"
        sudo mv "$tmpfile" "$install_dir/$BIN"
    else
        curl -sSfL "$url" -o "$tmpfile" 2>/dev/null || {
            echo "Need sudo or run as root"; exit 1
        }
        chmod +x "$tmpfile"
        mv "$tmpfile" "$install_dir/$BIN"
    fi

    echo "Installed $BIN $version to $install_dir/$BIN"
    echo ""
    echo "API key is built-in — no setup needed!"
    echo ""
    echo "Run TUI:"
    echo "  rustcli tui"
    echo ""
    echo "Or chat in terminal:"
    echo "  rustcli chat"
}

main "$@"
