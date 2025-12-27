+++
title = "CLI Reference"
description = "Complete reference for ddc commands and options"
weight = 25
+++

`ddc` is the dodeca command-line interface. It provides commands for building, serving, and managing your site.

## Commands Overview

| Command | Description |
|---------|-------------|
| `ddc build` | Build the site to the output directory |
| `ddc serve` | Build and serve with live reload |
| `ddc static` | Serve static files from a directory |
| `ddc clean` | Clear all caches |

## ddc build

Build the site to the output directory.

```
ddc build [path] [OPTIONS]
```

### Arguments

- `[path]` - Project directory (optional). If omitted, searches from the current directory upward for `.config/dodeca.kdl`.

### Options

| Option | Description |
|--------|-------------|
| `-c, --content <dir>` | Override the content directory |
| `-o, --output <dir>` | Override the output directory |
| `--tui` | Show TUI progress display |

### Examples

```bash
# Build the site in the current directory
ddc build

# Build a specific project
ddc build ~/my-site

# Build with custom output directory
ddc build -o dist

# Build with progress UI
ddc build --tui
```

## ddc serve

Build and serve the site with live reload. Changes to content, templates, or styles are automatically reflected in the browser.

```
ddc serve [path] [OPTIONS]
```

### Arguments

- `[path]` - Project directory (optional). If omitted, searches from the current directory upward for `.config/dodeca.kdl`.

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-c, --content <dir>` | Override the content directory | From config |
| `-o, --output <dir>` | Override the output directory | From config |
| `-a, --address <addr>` | Address to bind on | `127.0.0.1` |
| `-p, --port <port>` | Port to serve on | `4000` |
| `-P, --public` | Listen on all interfaces (LAN access) | Off |
| `--open` | Open browser after starting server | Off |
| `--no-tui` | Disable TUI (show plain logs) | Off |

### Examples

```bash
# Serve the site with defaults
ddc serve

# Serve on a specific port
ddc serve -p 8080

# Serve with LAN access and open browser
ddc serve --public --open

# Serve without the TUI
ddc serve --no-tui
```

### TUI Keyboard Shortcuts

When running with the TUI (default), these shortcuts are available:

| Key | Action |
|-----|--------|
| `?` | Toggle help overlay |
| `o` | Open first URL in browser |
| `p` | Toggle public/local mode |
| `d` | Toggle debug logging |
| `l` | Cycle log level |
| `f` | Cycle log filter presets |
| `F` | Enter custom log filter (RUST_LOG syntax) |
| `q` | Quit |
| `Ctrl+C` | Force quit |

## ddc static

Serve static files from a directory without any processing. Useful for quickly previewing built sites or serving any directory.

```
ddc static [path] [OPTIONS]
```

### Arguments

- `[path]` - Directory to serve (default: current directory)

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `-a, --address <addr>` | Address to bind on | `127.0.0.1` |
| `-p, --port <port>` | Port to serve on | `8080` |
| `-P, --public` | Listen on all interfaces (LAN access) | Off |
| `--open` | Open browser after starting server | Off |

### Examples

```bash
# Serve current directory
ddc static

# Serve a specific directory on a custom port
ddc static ./public -p 3000

# Serve with LAN access
ddc static --public
```

## ddc clean

Clear all caches. This removes the `.cache` directory which contains:

- Picante incremental build cache (`dodeca.bin`)
- Content-addressed storage for images
- Code execution cache

```
ddc clean [path]
```

### Arguments

- `[path]` - Project directory (optional). If omitted, searches from the current directory upward for `.config/dodeca.kdl`.

### Examples

```bash
# Clean caches for current project
ddc clean

# Clean caches for a specific project
ddc clean ~/my-site
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Log filter expression (e.g., `warn,dodeca=debug`) |
| `NO_COLOR` | Disable colored output when set |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Build error or failure |

## Project Discovery

When no `[path]` argument is provided, `ddc` searches for a project configuration by looking for `.config/dodeca.kdl` in:

1. The current working directory
2. Each parent directory, up to the filesystem root

This allows running `ddc` commands from anywhere within a project.
