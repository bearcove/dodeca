+++
title = "Installation"
description = "How to install dodeca"
weight = 10
+++

Ha.

The recommended way to install dodeca is from GitHub releases. Releases ship the `ddc` binary plus a set of helper binaries (`ddc-cell-*`) used for things like image processing, Sass, search indexing, etc.

## macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

Supported by the installer:
- macOS (Apple Silicon / `arm64`)
- Linux (`x86_64`)

## Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.ps1 | iex"
```

Supported by the installer:
- Windows (`x86_64`)

## From source

Since dodeca uses a plugin architecture, building from source requires multiple steps:

```bash
git clone https://github.com/bearcove/dodeca.git
cd dodeca
cargo xtask build
```

This will build the WASM components, plugins, and the main dodeca binary.

## Verify

After installation, verify it works:

```bash
ddc --help
```

## Updating

There is no in-app updater command in `ddc`. To update:
- If you installed from releases: re-run the installer command for your platform.
- If you built from source: pull changes and re-run `cargo xtask build`.
