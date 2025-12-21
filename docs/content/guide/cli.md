+++
title = "CLI"
description = "Commands and flags"
weight = 25
+++

`ddc` is the dodeca command-line interface.

## Commands

- `ddc build [path]` builds the site.
- `ddc serve [path]` builds the site and starts a dev server with live reload.
- `ddc clean [path]` clears caches.

`[path]` is optional. If provided, dodeca looks for `.config/dodeca.kdl` in that directory. If omitted, it searches from the current working directory upwards.

## Common options

### `ddc build`

- `-c, --content <dir>` override the content directory.
- `-o, --output <dir>` override the output directory.
- `--tui` show a progress UI.

### `ddc serve`

- `-c, --content <dir>` override the content directory.
- `-o, --output <dir>` override the output directory.
- `-a, --address <addr>` bind address (default `127.0.0.1`).
- `-p, --port <port>` preferred port (default starts at `4000`).
- `-P, --public` listen on all interfaces (LAN access).
- `--open` open a browser after the server starts.
- `--no-tui` disable the TUI (plain logs).
