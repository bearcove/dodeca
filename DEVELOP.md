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

## CI Workflow Generation

The release workflow and installer script are generated from Rust code, not hand-written YAML.

```sh
# Regenerate .github/workflows/release.yml and install.sh
cargo xtask ci

# Check if generated files are up to date (used in CI)
cargo xtask ci --check
```

The source of truth is `xtask/src/ci.rs`. Edit that file to change:
- Build targets
- Processor crates
- Workflow steps
- Installer script

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
