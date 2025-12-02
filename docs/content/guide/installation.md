+++
title = "Installation"
description = "How to install dodeca"
weight = 10
+++

## macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh
```

## Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.ps1 | iex"
```

## Homebrew

```bash
brew install bearcove/tap/dodeca
```

## From source

```bash
cargo install dodeca
```

## Verify

After installation, verify it works:

```bash
ddc --version
```
