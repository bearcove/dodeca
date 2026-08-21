//! Installer script generation for dodeca release artifacts.
//!
//! Both the POSIX shell (`install.sh`) and PowerShell (`install.ps1`) installers
//! are generated from `RELEASE_BASE_URL` here, so the default artifact location
//! stays a single source of truth. (The GitHub/Forgejo workflow generator that
//! used to live alongside these was removed; the release pipeline now runs on
//! Buildkite/private infrastructure.)

/// Base URL for release artifacts: the `bearcove-dist` Scaleway Object Storage
/// bucket (fr-par), under the `dodeca/releases/` prefix. dodeca is a bearcove
/// project, so it has its own bucket separate from Vixen's. Each release is
/// `<base>/<version>/dodeca-<platform>.tar.xz`; `<base>/latest` is a text file
/// holding the newest version string. Overridable at install time via
/// `DODECA_BASE_URL` (mirrors / testing).
///
/// Objects are uploaded public-read and served from the same S3 API endpoint
/// (`s3.fr-par.scw.cloud`); see scripts/publish-release.sh.
pub const RELEASE_BASE_URL: &str = "https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/releases";

/// Generate the shell installer script content.
pub fn generate_installer_script() -> String {
    // No Rust interpolation needed — everything below is shell. The default
    // base URL is injected once so the generator stays the single source.
    format!(
        r##"#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/install.sh | sh

set -eu

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override BASE_URL for a mirror or local testing; DODECA_VERSION pins a
# specific version (otherwise the `latest` pointer is read).
BASE_URL="${{DODECA_BASE_URL:-{base_url}}}"

# Detect platform (only linux-x64 and macos-arm64 are supported)
detect_platform() {{
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
}}

# Read the `latest` pointer (a text file holding the newest version string).
get_latest_version() {{
    curl -fsSL "$BASE_URL/latest"
}}

main() {{
    local platform version archive_name url install_dir

    platform="$(detect_platform)"
    version="${{DODECA_VERSION:-$(get_latest_version)}}"
    archive_name="dodeca-$platform.tar.xz"
    url="$BASE_URL/$version/$archive_name"
    install_dir="${{DODECA_INSTALL_DIR:-$HOME/.cargo/bin}}"

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

    # Copy browser JS/WASM assets beside the binary. ddc also supports
    # DODECA_ASSETS_DIR for custom package layouts.
    if [ -d "$tmpdir/dodeca-assets" ]; then
        rm -rf "$install_dir/dodeca-assets"
        cp -R "$tmpdir/dodeca-assets" "$install_dir/"
    else
        echo "warning: archive did not contain dodeca-assets; browser search and DevTools assets will be missing" >&2
        echo "warning: run 'ddc assets' after installation for lookup paths and repair commands" >&2
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
}}

main "$@"
"##,
        base_url = RELEASE_BASE_URL
    )
}

/// Generate the PowerShell installer script content.
pub fn generate_powershell_installer() -> String {
    format!(
        r##"# Installer for dodeca
# Usage: powershell -ExecutionPolicy Bypass -c "irm https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/install.ps1 | iex"

$ErrorActionPreference = 'Stop'

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override with $env:DODECA_BASE_URL; $env:DODECA_VERSION pins a version.
$BaseUrl = if ($env:DODECA_BASE_URL) {{ $env:DODECA_BASE_URL }} else {{ "{base_url}" }}

function Get-Architecture {{
    $arch = [System.Environment]::Is64BitOperatingSystem
    if ($arch) {{
        return "x86_64"
    }} else {{
        Write-Error "Only x64 architecture is supported on Windows"
        exit 1
    }}
}}

function Get-LatestVersion {{
    try {{
        return (Invoke-RestMethod -Uri "$BaseUrl/latest").Trim()
    }} catch {{
        Write-Error "Failed to get latest version: $_"
        exit 1
    }}
}}

function Main {{
    $arch = Get-Architecture
    $version = if ($env:DODECA_VERSION) {{ $env:DODECA_VERSION }} else {{ Get-LatestVersion }}
    $archiveName = "dodeca-x86_64-pc-windows-msvc.zip"
    $url = "$BaseUrl/$version/$archiveName"

    # Default install location
    $installDir = if ($env:DODECA_INSTALL_DIR) {{
        $env:DODECA_INSTALL_DIR
    }} else {{
        Join-Path $env:LOCALAPPDATA "dodeca"
    }}

    Write-Host "Installing dodeca $version for Windows x64..."
    Write-Host "  Archive: $url"
    Write-Host "  Install dir: $installDir"

    # Create install directory
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null

    # Download and extract
    $tempDir = Join-Path $env:TEMP "dodeca-install-$(New-Guid)"
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

    try {{
        Write-Host "Downloading..."
        $archivePath = Join-Path $tempDir "archive.zip"
        Invoke-WebRequest -Uri $url -OutFile $archivePath

        Write-Host "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

        Write-Host "Installing..."
        Copy-Item -Path (Join-Path $tempDir "ddc.exe") -Destination $installDir -Force

        $assetsDir = Join-Path $tempDir "dodeca-assets"
        if (Test-Path $assetsDir) {{
            $installedAssets = Join-Path $installDir "dodeca-assets"
            if (Test-Path $installedAssets) {{
                Remove-Item -Recurse -Force $installedAssets
            }}
            Copy-Item -Path $assetsDir -Destination $installDir -Recurse -Force
        }} else {{
            Write-Warning "Archive did not contain dodeca-assets; browser search and DevTools assets will be missing."
            Write-Warning "Run 'ddc assets' after installation for lookup paths and repair commands."
        }}

        Write-Host ""
        Write-Host "Successfully installed dodeca to $installDir\ddc.exe"
        Write-Host ""

        # Check if install_dir is in PATH
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$installDir*") {{
            Write-Host "NOTE: $installDir is not in your PATH."
            Write-Host "Adding $installDir to your user PATH..."

            try {{
                $newPath = if ($userPath) {{ "$userPath;$installDir" }} else {{ $installDir }}
                [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
                Write-Host "Successfully added to PATH. You may need to restart your terminal."
            }} catch {{
                Write-Host "Failed to add to PATH automatically. Please add it manually:"
                Write-Host "  1. Open System Properties > Environment Variables"
                Write-Host "  2. Add '$installDir' to your user PATH variable"
            }}
            Write-Host ""
        }}
    }} finally {{
        # Cleanup
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}

Main
"##,
        base_url = RELEASE_BASE_URL
    )
}
