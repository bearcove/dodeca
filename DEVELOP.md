# Development Guide

## Building

```sh
# Build everything (WASM + plugins + dodeca)
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
- Plugin list
- Workflow steps
- Installer script

## Release Process

Releases are triggered by pushing a version tag:

```sh
git tag v0.3.0
git push origin v0.3.0
```

This will:
1. Build `ddc` + plugins for all 5 targets
2. Create archives with `ddc` and `plugins/` directory
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

## Cell Architecture

Cells are separate processes that handle specialized tasks (image processing, markdown
rendering, terminal recording, etc.). They communicate with the main `ddc` process via
roam RPC over shared memory.

### How Cells Work

1. **Lazy Spawning**: Cells are NOT started at `ddc` startup. They're spawned on-demand
   when first accessed via `client_async<T>()`.

2. **Initialization Flow**:
   - `term_cell().await` calls `Host::get().client_async::<TermRecorderClient>()`
   - `client_async` calls `ensure_cell_registry_initialized()` (idempotent)
   - First call to `ensure_cell_registry_initialized()` runs `init_cells()` which:
     - Creates SHM path in `/tmp/roam-shm/`
     - Builds a `MultiPeerHostDriver` with NO peers initially
     - Stores the driver handle in `Host::get().driver_handle`
     - Registers all cells as "pending" (not spawned yet)
     - Spawns the driver task
   - Back in `client_async`, if cell is not ready, calls `spawn_pending_cell()`
   - `spawn_pending_cell()` creates a peer on the driver and spawns the cell binary
   - Cell binary connects back via SHM and reports ready

3. **No Boot Function Needed**: You don't need to call `cells::boot()` or similar.
   Just call the cell accessor (e.g., `term_cell().await`) and everything happens
   automatically.

### Cell Structure

Each cell has two crates:

- `cells/cell-X-proto/` - Protocol definition with:
  - Data structures using `#[derive(Facet)]` for serialization
  - Service trait using `#[roam::service]` macro
  - Custom result enums (not `Result<T>`)
  - Minimal dependencies (just facet + roam)

- `cells/cell-X/` - Implementation with:
  - Binary target `ddc-cell-X` that implements the proto trait
  - Uses `dodeca_cell_runtime::cell_service!()` macro
  - Uses `dodeca_cell_runtime::run_cell!()` macro for main()

### Adding a New Cell

1. Create `cells/cell-mycell-proto/Cargo.toml`:
   ```toml
   [package]
   name = "cell-mycell-proto"
   version = "0.6.1"
   edition = "2024"

   [dependencies]
   facet.workspace = true
   roam.workspace = true
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
   #[roam::service]
   pub trait MyService {
       async fn do_thing(&self, config: MyConfig) -> MyResult;
   }
   ```

3. Create `cells/cell-mycell/Cargo.toml`:
   ```toml
   [package]
   name = "cell-mycell"
   version = "0.6.1"
   edition = "2024"

   [[bin]]
   name = "ddc-cell-mycell"
   path = "src/main.rs"

   [dependencies]
   cell-mycell-proto = { path = "../cell-mycell-proto" }
   dodeca-cell-runtime = { path = "../../crates/dodeca-cell-runtime" }
   # ... other deps
   ```

4. Create `cells/cell-mycell/src/main.rs`:
   ```rust
   use cell_mycell_proto::*;
   use dodeca_cell_runtime::{cell_service, run_cell};

   struct MyServiceImpl;

   cell_service!(MyServiceDispatcher for MyServiceImpl as MyService {
       async fn do_thing(&self, config: MyConfig) -> MyResult {
           // implementation
       }
   });

   fn main() -> Result<(), Box<dyn std::error::Error>> {
       run_cell!("mycell", |_handle| MyServiceDispatcher::new(MyServiceImpl))
   }
   ```

5. Register in `crates/dodeca/src/cells.rs`:
   ```rust
   // Add import
   use cell_mycell_proto::{MyConfig, MyServiceClient, MyResult};

   // Add to define_plugins! macro call
   CellDef::new("mycell"),  // or .inherit_stdio() if it needs terminal access

   // Add impl_cell_client! in host.rs
   impl_cell_client!(cell_mycell_proto::MyServiceClient, "mycell");

   // Add accessor and wrapper
   cell_client_accessor!(mycell_cell, "mycell", MyServiceClient);

   pub async fn do_mycell_thing(config: MyConfig) -> Result<MyResult, eyre::Error> {
       let client = mycell_cell()
           .await
           .ok_or_else(|| eyre::eyre!("Mycell not available"))?;
       client
           .do_thing(config)
           .await
           .map_err(|e| eyre::eyre!("RPC error: {:?}", e))
   }
   ```

6. Add dependency to `crates/dodeca/Cargo.toml`:
   ```toml
   cell-mycell-proto = { path = "../../cells/cell-mycell-proto" }
   ```

### Debugging Cells

- Set `DODECA_QUIET=1` when TUI is active to suppress cell output
- Send `SIGUSR1` to dodeca process to dump hub transport diagnostics
- Cell binaries print debug info prefixed with `[cell-X]` during startup
