#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://raw.githubusercontent.com/bearcove/dodeca/main/install.sh | sh

set -eu

REPO="bearcove/dodeca"

# Detect platform
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64) echo "x86_64-unknown-linux-gnu" ;;
                aarch64) echo "aarch64-unknown-linux-gnu" ;;
                *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                x86_64) echo "Intel Macs (x86_64) are not supported. Use an ARM Mac or build from source." >&2; exit 1 ;;
                arm64) echo "aarch64-apple-darwin" ;;
                *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac
}

# Get latest release version
get_latest_version() {
    curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/'
}

main() {
    local platform version archive_name url install_dir

    platform="$(detect_platform)"
    version="${DODECA_VERSION:-$(get_latest_version)}"
    archive_name="dodeca-$platform.tar.xz"
    url="https://github.com/$REPO/releases/download/$version/$archive_name"
    install_dir="${DODECA_INSTALL_DIR:-$HOME/.cargo/bin}"

    echo "Installing dodeca $version for $platform..."
    echo "  Archive: $url"
    echo "  Install dir: $install_dir"

    # Create install directory
    mkdir -p "$install_dir"

    # Download and extract
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap "rm -rf '$tmpdir'" EXIT

    echo "Downloading..."
    curl -fsSL "$url" -o "$tmpdir/archive.tar.xz"

    echo "Extracting..."
    tar -xJf "$tmpdir/archive.tar.xz" -C "$tmpdir"

    echo "Installing..."
    # Copy main binary
    cp "$tmpdir/ddc" "$install_dir/"
    chmod +x "$install_dir/ddc"

    # Copy rapace plugin binaries (dodeca-mod-*)
    for plugin in "$tmpdir"/dodeca-mod-*; do
        if [ -f "$plugin" ]; then
            cp "$plugin" "$install_dir/"
            chmod +x "$install_dir/$(basename "$plugin")"
        fi
    done

    # Copy cdylib plugins if present (legacy)
    if [ -d "$tmpdir/plugins" ]; then
        cp "$tmpdir/plugins/"* "$install_dir/plugins/" 2>/dev/null || true
    fi

    echo ""
    echo "Successfully installed dodeca to $install_dir/ddc"
    echo ""

    # Check if install_dir is in PATH
    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *)
            echo "NOTE: $install_dir is not in your PATH."
            echo "Add this to your shell profile:"
            echo ""
            echo "  export PATH=\"\$PATH:$install_dir\""
            echo ""
            ;;
    esac
}

main "$@"
