# Claude Code Instructions for dodeca

## Organizations

dodeca is organized in one main binary (`dodeca` package, builds to `ddc`) and a lot
of different cells.

It uses different important technical components:

  * rapace RPC, usually available at `../rapace`, see https://rapace.bearcove.eu
  * picante query system, an async-friendly salsa-like, usually available at `../picante`

## Cells Architecture

Cells are separate processes that handle specialized tasks (image processing, markdown rendering,
HTML manipulation, etc.). They communicate with the main dodeca process via RPC over shared memory.

### Structure

Each cell has two crates:

  * `cell-X-proto` - Protocol definition with:
    - Data structures using `#[derive(Facet)]` for serialization
    - Service trait using `#[rapace::service]` macro
    - Custom result enums (not `Result<T>`)
    - Minimal dependencies (just facet + rapace)

  * `cell-X` - Implementation with:
    - Binary target that implements the proto trait
    - Uses `dodeca_cell_runtime::cell_service!()` macro
    - Uses `dodeca_cell_runtime::run_cell!()` macro for main()

### Adding a New Cell

1. Create `cells/cell-mycell-proto/` with service definition
2. Create `cells/cell-mycell/` with implementation
3. Both are automatically built as part of the workspace
4. Register in `crates/dodeca/src/cells.rs` using `define_plugins!` macro

### Debugging Cells

  * Set `RAPACE_DEBUG=1` to see cell startup debug output
  * Set `DODECA_QUIET=1` when TUI is active to suppress cell output
  * Send `SIGUSR1` to dodeca process to dump hub transport diagnostics

## Building and Running

**IMPORTANT**: `cargo build` only compiles the code - it does NOT install the `ddc` binary to a usable location.

### To run a debug build:
```bash
cargo xtask run -- serve ../styx --no-tui
```

This will:
1. Build the dodeca-devtools WASM bundle (via build.rs)
2. Build all cells (they're in the workspace)
3. Build the main dodeca binary
4. Run the binary with the provided arguments

### Alternative: Install and run
```bash
cargo xtask install   # Installs to ~/.cargo/bin/ddc
ddc serve ../styx --no-tui
```

## Testing

The dodeca repository itself is a dodeca website (it has `.config/dodeca.yaml`).
To test, run from the dodeca directory:

```bash
cargo xtask run -- build   # Build the site
cargo xtask run -- serve   # Serve with TUI (q to quit)
```

