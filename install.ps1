# Installer for dodeca
# Usage: powershell -ExecutionPolicy Bypass -c "irm https://vixen-misc.s3-website.fr-par.scw.cloud/dodeca/install.ps1 | iex"

$ErrorActionPreference = 'Stop'

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override with $env:DODECA_BASE_URL; $env:DODECA_VERSION pins a version.
$BaseUrl = if ($env:DODECA_BASE_URL) { $env:DODECA_BASE_URL } else { "https://vixen-misc.s3-website.fr-par.scw.cloud/dodeca/releases" }

function Get-Architecture {
    $arch = [System.Environment]::Is64BitOperatingSystem
    if ($arch) {
        return "x86_64"
    } else {
        Write-Error "Only x64 architecture is supported on Windows"
        exit 1
    }
}

function Get-LatestVersion {
    try {
        return (Invoke-RestMethod -Uri "$BaseUrl/latest").Trim()
    } catch {
        Write-Error "Failed to get latest version: $_"
        exit 1
    }
}

function Main {
    $arch = Get-Architecture
    $version = if ($env:DODECA_VERSION) { $env:DODECA_VERSION } else { Get-LatestVersion }
    $archiveName = "dodeca-x86_64-pc-windows-msvc.zip"
    $url = "$BaseUrl/$version/$archiveName"

    # Default install location
    $installDir = if ($env:DODECA_INSTALL_DIR) {
        $env:DODECA_INSTALL_DIR
    } else {
        Join-Path $env:LOCALAPPDATA "dodeca"
    }

    Write-Host "Installing dodeca $version for Windows x64..."
    Write-Host "  Archive: $url"
    Write-Host "  Install dir: $installDir"

    # Create install directory
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $pluginsDir = Join-Path $installDir "plugins"
    New-Item -ItemType Directory -Force -Path $pluginsDir | Out-Null

    # Download and extract
    $tempDir = Join-Path $env:TEMP "dodeca-install-$(New-Guid)"
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

    try {
        Write-Host "Downloading..."
        $archivePath = Join-Path $tempDir "archive.zip"
        Invoke-WebRequest -Uri $url -OutFile $archivePath

        Write-Host "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

        Write-Host "Installing..."
        Copy-Item -Path (Join-Path $tempDir "ddc.exe") -Destination $installDir -Force

        $tempPluginsDir = Join-Path $tempDir "plugins"
        if (Test-Path $tempPluginsDir) {
            Copy-Item -Path (Join-Path $tempPluginsDir "*") -Destination $pluginsDir -Force
        }

        Write-Host ""
        Write-Host "Successfully installed dodeca to $installDir\ddc.exe"
        Write-Host ""

        # Check if install_dir is in PATH
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$installDir*") {
            Write-Host "NOTE: $installDir is not in your PATH."
            Write-Host "Adding $installDir to your user PATH..."

            try {
                $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
                [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
                Write-Host "Successfully added to PATH. You may need to restart your terminal."
            } catch {
                Write-Host "Failed to add to PATH automatically. Please add it manually:"
                Write-Host "  1. Open System Properties > Environment Variables"
                Write-Host "  2. Add '$installDir' to your user PATH variable"
            }
            Write-Host ""
        }
    } finally {
        # Cleanup
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Main
