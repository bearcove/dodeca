+++
title = "Installation"
weight = 10
+++

## Installer (macOS / Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

Supported platforms:

- macOS (Apple Silicon / `arm64`)
- Linux (`x86_64`)

## From source

```bash
git clone https://github.com/bearcove/dodeca.git
cd dodeca
cargo xtask install
```

This builds everything and installs it to `~/.cargo/bin/`.

## Windows

Not currently supported. [roam](https://github.com/bearcove/roam), the RPC framework dodeca uses for cell communication, does not work on Windows yet. There are no Windows binaries.
