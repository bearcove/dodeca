#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/install.sh | sh

set -eu

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override BASE_URL for a mirror or local testing; DODECA_VERSION pins a
# specific version (otherwise the `latest` pointer is read).
BASE_URL="${DODECA_BASE_URL:-https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/releases}"

# Detect platform (only linux-x64 and macos-arm64 are supported)
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64) echo "x86_64-unknown-linux-gnu" ;;
                *) echo "Unsupported Linux architecture: $arch (only x86_64 supported)" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64) echo "aarch64-apple-darwin" ;;
                *) echo "Unsupported macOS architecture: $arch (only arm64 supported)" >&2; exit 1 ;;
            esac
            ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac
}

# Read the `latest` pointer (a text file holding the newest version string).
get_latest_version() {
    curl -fsSL "$BASE_URL/latest"
}

main() {
    local platform version archive_name url install_dir

    platform="$(detect_platform)"
    version="${DODECA_VERSION:-$(get_latest_version)}"
    archive_name="dodeca-$platform.tar.xz"
    url="$BASE_URL/$version/$archive_name"
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

    # Copy cell cdylibs (libddc_cell_*)
    for plugin in "$tmpdir"/libddc_cell_*; do
        if [ -f "$plugin" ]; then
            cp "$plugin" "$install_dir/"
        fi
    done

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
