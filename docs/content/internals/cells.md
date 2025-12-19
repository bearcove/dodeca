+++
title = "Cells"
description = "Cell systems for extending dodeca functionality"
weight = 50
+++

## Introduction

Dodeca uses cells to keep the core binary small and fast to link. Heavy dependencies live in cells, which compile and link independently.

There are currently two cell systems:

| System | Type | Communication | Status |
|--------|------|---------------|--------|
| **Plugcard** | Dynamic library (.so/.dylib/.dll) | Serialized method calls | Legacy, being phased out |
| **Rapace** | Subprocess binary | Shared memory (zero-copy) | Active development |

All cells are being migrated to **rapace**.

---

## Plugcard (Legacy)

A dynamic library cell system inspired by [postcard-rpc](https://github.com/jamesmunns/postcard-rpc).

Plugcard cells are dynamic libraries loaded into the main process. They use serialized method calls across the FFI boundary.

### How It Works

The `#[plugcard]` attribute macro transforms your function into a cell method:

1. **Input struct** - Arguments are bundled into a generated struct with serde derives
2. **FFI wrapper** - An `extern "C"` function that deserializes input, calls your function, serializes output
3. **Registration** - A static `MethodSignature` is registered in a distributed slice

### Quick Start

Add dependencies:

```toml
[dependencies]
plugcard = { path = "../plugcard" }
linkme = "0.3"
postcard-schema = { version = "0.1", features = ["derive", "alloc"] }

[lib]
crate-type = ["cdylib", "rlib"]
```

Mark functions with `#[plugcard]`:

```rust,noexec
use plugcard::plugcard;

#[plugcard]
pub fn reverse_string(input: String) -> String {
    input.chars().rev().collect()
}

#[plugcard]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

The macro generates all FFI wrappers and registration code.

### Generated Code

For a function like:

```rust,noexec
#[plugcard]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

The macro generates:

```rust,noexec
// Original function preserved
pub fn add(a: i32, b: i32) -> i32 { a + b }

// Input composite type
#[derive(Serialize, Deserialize, Schema)]
struct __PlugcardInput_add { pub a: i32, pub b: i32 }

// C-compatible wrapper
unsafe extern "C" fn __plugcard_wrapper_add(data: *mut MethodCallData) {
    // Deserialize input
    let input: __PlugcardInput_add = postcard::from_bytes(...)?;
    // Call function
    let result = add(input.a, input.b);
    // Serialize output
    postcard::to_slice(&result, ...)?;
}

// Auto-register in distributed slice
#[distributed_slice(METHODS)]
static __PLUGCARD_SIG_add: MethodSignature = MethodSignature {
    key: compute_key("add", ...),
    name: "add",
    input_schema: ...,
    output_schema: ...,
    call: __plugcard_wrapper_add,
};
```

### API Reference

#### `MethodSignature`

```rust,noexec
pub struct MethodSignature {
    pub key: u64,           // Unique key from name + schemas
    pub name: &'static str, // Human-readable method name
    pub input_schema: &'static NamedType,
    pub output_schema: &'static NamedType,
    pub call: unsafe extern "C" fn(*mut MethodCallData),
}
```

#### `MethodCallData`

The FFI boundary structure:

```rust,noexec
#[repr(C)]
pub struct MethodCallData {
    pub key: u64,
    pub input_ptr: *const u8,
    pub input_len: usize,
    pub output_ptr: *mut u8,
    pub output_cap: usize,
    pub output_len: usize,  // Set by callee
    pub result: MethodCallResult,
}
```

#### `MethodCallResult`

```rust,noexec
#[repr(C)]
pub enum MethodCallResult {
    Success,
    DeserializeError,
    SerializeError,
    MethodError,
    UnknownMethod,
}
```

### Method Keys

Method keys are computed at compile time using FNV-1a hash of:
- Method name
- Input schema name
- Output schema name

This ensures type-safe dispatch: if schemas change, keys change.

### Crate Structure

- **plugcard** - Core types and runtime
- **plugcard-macros** - The `#[plugcard]` proc macro (uses [unsynn](https://docs.rs/unsynn) for parsing)
- **dodeca-baseline** - Example cell with test functions

---

## Rapace (Recommended)

Rapace cells are standalone executables that communicate with the host via shared memory (SHM) using the [rapace](https://rapace.bearcove.eu/) framework. The SHM transport enables zero-copy data transfer between the host and cell processes.

### Hub Architecture

All cells share a single SHM "hub" file with variable-size slot allocation:

| Slot Size | Count | Purpose |
|-----------|-------|---------|
| 1KB | 1024 | Small RPC args |
| 16KB | 256 | Typical payloads |
| 256KB | 32 | Images, CSS |
| 4MB | 8 | Compressed fonts |
| 16MB | 4 | Decompressed fonts |

Each cell gets:
- A unique `peer_id` assigned by the host
- Its own ring pair (send/recv) within the shared SHM
- A socketpair doorbell for cross-process wakeup

For detailed architecture, see [SHM Hub Architecture](/docs/SHM-HUB-ARCHITECTURE.md).

### Benefits

- **Zero-copy performance** - Content transfers directly through shared memory without copying
- **Process isolation** - Cells run in separate processes, improving stability
- **Bidirectional RPC** - Both host and cell can initiate calls (via `RpcSession`)
- **TCP tunneling** - Browser connections are accepted by host and tunneled through to cell
- **Async support** - Full async/await with independent runtimes per cell
- **Variable-size slots** - Font decompression can use 16MB slots while small RPC uses 1KB
- **Shared memory pool** - All cells share a ~109MB pool instead of separate allocations

### Current Rapace Cells

- `cell-http` - HTTP dev server with WebSocket support for live reload
- `cell-arborium` - Syntax highlighting using tree-sitter via arborium library

### Directory Organization

Rapace cells live in a separate `cells/` directory outside the main workspace. This separation enables cells to link independently without triggering rebuilds of the core binary.

**Directory structure:**

```
cells/
├── cell-http/                 # HTTP dev server cell
│   ├── Cargo.toml           # package: cell-http, bin: dodeca-cell-http
│   └── src/main.rs
├── cell-http-proto/          # HTTP cell protocol definitions
│   ├── Cargo.toml           # package: cell-http-proto
│   └── src/lib.rs
├── cell-arborium/            # Syntax highlighting cell
│   ├── Cargo.toml           # package: cell-arborium, bin: dodeca-cell-arborium
│   └── src/main.rs
└── cell-arborium-proto/      # Syntax highlighting protocol definitions
    ├── Cargo.toml           # package: cell-arborium-proto
    └── src/lib.rs
```

**Naming convention for new mods:**

Each rapace cell follows this consistent pattern:

1. **Cell binary**: `cells/mod-{name}/`
   - Cargo package name: `mod-{name}`
   - Binary name: `dodeca-mod-{name}` (defined in `[[bin]]` section)

2. **Protocol crate**: `cells/mod-{name}-proto/`
   - Cargo package name: `mod-{name}-proto`
   - Contains `#[rapace::service]` trait definitions

3. **Dependencies**:
   - Cell depends on its protocol via relative path: `{ path = "../mod-{name}-proto" }`
   - Both cell and protocol depend on rapace framework crates from git

4. **Workspace exclusion**:
   - The root `Cargo.toml` excludes mods: `exclude = ["cells/*"]`
   - This allows mods to have independent dependency versions and compile separately

### Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                          Core dodeca                                  │
│  ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌─────────────────────────────┐│
│  │  Picante  │ │Markdown │ │ Template │ │      Cell Host            ││
│  │(queries)│ │ Parser  │ │  Engine  │ │  - TCP listener (browsers)  ││
│  └─────────┘ └─────────┘ └──────────┘ │  - RpcSession + dispatcher  ││
│                                        │  - ContentService impl      ││
│                                        └──────────────┬──────────────┘│
└───────────────────────────────────────────────────────┼───────────────┘
                                                        │ SHM (zero-copy)
                                                        ▼
                               ┌─────────────────────────────────────┐
                               │        dodeca-cell-http              │
                               │   - Internal axum HTTP server       │
                               │   - TcpTunnel service (host→cell) │
                               │   - ContentService client (cell→host)│
                               │   - WebSocket for devtools          │
                               └─────────────────────────────────────┘
```

### Communication Flow

Host and cell communicate bidirectionally via shared memory:

```
Browser                Host (dodeca)                Cell (dodeca-cell-http)
   │                        │                                │
   │── TCP connect ────────▶│                                │
   │                        │── TcpTunnel.open() ───────────▶│
   │                        │◀── tunnel handle ──────────────│
   │                        │                                │
   │── HTTP request ───────▶│── tunnel chunk ───────────────▶│
   │                        │                                │ (internal axum)
   │                        │                                │
   │                        │◀── find_content("/foo") ───────│
   │                        │ (queries Picante DB)             │
   │                        │── ServeContent::Html {...} ───▶│
   │                        │                                │
   │                        │◀── tunnel response chunk ──────│
   │◀── HTTP response ──────│                                │
   │                        │                                │
```

The host accepts browser TCP connections and tunnels them through to the cell via `TcpTunnel`. The cell processes HTTP requests internally and calls back to the host for content via `ContentService`.

### Protocol Definition

Rapace cells use trait-based protocol definitions with the `#[rapace::service]` macro:

```rust,noexec
#[rapace::service]
pub trait ContentService {
    async fn find_content(&self, path: String) -> ServeContent;
    async fn get_scope(&self, route: String, path: Vec<String>) -> Vec<ScopeEntry>;
    async fn eval_expression(&self, route: String, expression: String) -> EvalResult;
    async fn open_ws_tunnel(&self) -> u64;
}
```

The macro generates:
- Client types for making RPC calls
- Server types for handling RPC calls
- Serialization/deserialization code

### Creating a Rapace Cell

To create a new rapace cell, follow this structure (using `example` as the cell name):

1. **Create the protocol crate** at `cells/mod-example-proto/`
   - Package name: `mod-example-proto`
   - Define your RPC traits using `#[rapace::service]`

2. **Create the cell binary** at `cells/mod-example/`
   - Package name: `mod-example`
   - Binary name: `dodeca-mod-example`
   - Depend on `mod-example-proto` via relative path

3. **Implement the server side** in the host
   - The host implements the traits defined in the protocol crate
   - Register the service with the RpcSession

4. **Cell implementation**
   - Connect to the RpcSession in the host via shared memory
   - Call the host's service methods as needed

**Key points:**
- Each cell is completely independent; use relative path dependencies for its protocol crate
- The root workspace excludes `cells/*`, so cells build independently from the core binary
- Cells can have their own dependency versions since they're not in the workspace

See `cells/cell-http/` and `cells/cell-arborium/` for complete examples.

---

## Why Cells?

The primary motivation is **link speed**. Rust's incremental compilation is fast, but linking a large binary with many dependencies is slow. By moving functionality into cells:

- The main `dodeca` binary stays small and links fast
- Cells compile and link independently
- Changing a cell doesn't require relinking the main binary
- Heavy dependencies (image processing, font subsetting, HTTP) live in cells

This dramatically improves iteration speed during development.

## Future Plans

More functionality will move to rapace cells:

- `http-client` - For link checking and external fetches
- `search` - Full-text search indexing (replacing pagefind)
- `image-processing` - Image optimization and conversion
- `font-subsetting` - Web font optimization

Cells can depend on each other through the message-passing system, keeping each focused and avoiding duplicated dependencies.
