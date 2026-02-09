+++
title = "Installation"
weight = 10
+++

## From GitHub Releases (recommended)

The installer ships the `ddc` binary plus all helper binaries (`ddc-cell-*`) used for image processing, Sass compilation, font subsetting, and more.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

Supported platforms:

- macOS (Apple Silicon / `arm64`)
- Linux (`x86_64`)

## From Source

Requires a Rust toolchain.

```bash
git clone https://github.com/bearcove/dodeca.git
cd dodeca
cargo xtask install
```

This installs `ddc` and all cell binaries to `~/.cargo/bin/`.
