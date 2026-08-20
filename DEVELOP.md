# Development Guide

## Building

```sh
# Build everything (WASM + dodeca)
cargo xtask build

# Build in release mode
cargo xtask build --release

# Run ddc after building
cargo xtask run -- serve

# Install to ~/.cargo/bin
cargo xtask install
```

`cargo xtask install` builds the release binary, installs `ddc` to `~/.cargo/bin`,
builds the browser JS/WASM assets (search runtime, DevTools runtime + UI), and
stages them into `~/.cargo/bin/dodeca-assets/` so the installed `ddc` can build
sites out of the box. It verifies the staged assets with `ddc assets --packaged
--fail` and fails the install if any is missing.

Building the browser assets requires:

- `wasm-pack` — installed automatically via `cargo install wasm-pack --locked`
  when missing
- `pnpm` — installed automatically via `npm install -g pnpm` when missing
- the `wasm32-unknown-unknown` Rust target (added by `build-browser-assets.sh`)

## CI Workflow

CI is a hand-written, minimal GitHub Actions workflow: `.github/workflows/ci.yml`
runs `cargo check` and `cargo clippy` on Blacksmith 2-core Linux runners for
pushes/pull requests/merge-group on `main`. Full and private pipelines run on
Buildkite/own infrastructure.

The installer scripts (`install.sh` / `install.ps1`) are generated from Rust code
in `xtask/src/installer.rs`, not hand-written:

```sh
# Regenerate the installers
cargo xtask generate-installer install.sh
cargo xtask generate-ps1-installer install.ps1
```

The source of truth for the installer scripts is `xtask/src/installer.rs`
(`RELEASE_BASE_URL`). Edit that file to change the installer generation.

## Release Process

Releases are triggered by pushing a version tag:

```sh
git tag v0.3.0
git push origin v0.3.0
```

This will:
1. Build `ddc` for all release targets
2. Create archives with the `ddc` binary
3. Generate checksums
4. Create a GitHub release with all assets

## Installing from Release

```sh
# Install latest release
curl -fsSL https://raw.githubusercontent.com/bearcove/dodeca/main/install.sh | sh

# Install specific version
DODECA_VERSION=v0.3.0 curl -fsSL https://raw.githubusercontent.com/bearcove/dodeca/main/install.sh | sh

# Install to custom directory
DODECA_INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/bearcove/dodeca/main/install.sh | sh
```

## Processor Architecture

Dodeca now builds as one `ddc` binary. The old dynamic cell boundary has been
collapsed into direct Rust calls through `crates/dodeca/src/cells.rs`.

The source tree still uses `cells/cell-*` crate names for continuity, but they
are processor crates linked into `ddc`, not helper binaries discovered on
`PATH`, spawned on demand, or connected over shared memory.

### Processor Structure

Each processor usually has two crates:

- `cells/cell-X-proto/` - Protocol definition with:
  - Data structures using `#[derive(Facet)]` for serialization
  - Service trait used by the in-process implementation
  - Custom result enums (not `Result<T>`)
  - Minimal dependencies

- `cells/cell-X/` - Implementation with:
  - A Rust library target that implements the proto trait
  - Dependencies needed by that processor
  - No standalone runtime entrypoint for production dispatch

### Adding a New Processor

1. Create `cells/cell-mycell-proto/Cargo.toml`:
   ```toml
   [package]
   name = "cell-mycell-proto"
   version = "0.6.1"
   edition = "2024"

   [dependencies]
   facet.workspace = true
   ```

2. Create `cells/cell-mycell-proto/src/lib.rs`:
   ```rust
   use facet::Facet;

   #[derive(Debug, Clone, Facet)]
   pub struct MyConfig { /* ... */ }

   #[derive(Debug, Clone, Facet)]
   #[repr(u8)]
   pub enum MyResult {
       Success { data: String },
       Error { message: String },
   }

   #[allow(async_fn_in_trait)]
   pub trait MyService {
       async fn do_thing(&self, config: MyConfig) -> MyResult;
   }
   ```

3. Create `cells/cell-mycell/Cargo.toml`:
   ```toml
   [package]
   autobins = false
   name = "cell-mycell"
   version = "0.6.1"
   edition = "2024"

   [lib]
   name = "ddc_cell_mycell"
   crate-type = ["rlib"]
   path = "src/main.rs"

   [dependencies]
   cell-mycell-proto = { path = "../cell-mycell-proto" }
   # ... other deps
   ```

4. Create `cells/cell-mycell/src/main.rs`:
   ```rust
   use cell_mycell_proto::*;

   pub struct MyServiceImpl;

   impl MyService for MyServiceImpl {
       async fn do_thing(&self, config: MyConfig) -> MyResult {
           // implementation
       }
   }
   ```

5. Register in `crates/dodeca/src/cells.rs`:
   ```rust
   // Add import
   use cell_mycell_proto::{MyConfig, MyResult, MyService};

   pub async fn do_mycell_thing(config: MyConfig) -> MyResult {
       ddc_cell_mycell::MyServiceImpl.do_thing(config).await
   }
   ```

6. Add dependency to `crates/dodeca/Cargo.toml`:
   ```toml
   cell-mycell-proto = { path = "../../cells/cell-mycell-proto" }
   cell-mycell = { path = "../../cells/cell-mycell" }
   ```
