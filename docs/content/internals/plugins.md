+++
title = "Plugins"
description = "Dynamic library plugin system with serialized method calls"
weight = 50
+++

A dynamic library plugin system for Rust, inspired by [postcard-rpc](https://github.com/jamesmunns/postcard-rpc).

## Overview

Plugcard allows you to expose Rust functions as plugin methods that can be called across dynamic library boundaries. It handles:

- **Serialization** - Input/output automatically serialized via [postcard](https://docs.rs/postcard)
- **Schema introspection** - Methods carry type schemas for validation
- **Auto-registration** - Methods register themselves via [linkme](https://docs.rs/linkme) distributed slices
- **FFI safety** - Clean C-compatible interface across the boundary

## Quick Start

### 1. Add dependencies

```toml
[dependencies]
plugcard = { path = "../plugcard" }
linkme = "0.3"
postcard-schema = { version = "0.1", features = ["derive", "alloc"] }

[lib]
crate-type = ["cdylib", "rlib"]
```

### 2. Mark functions with `#[plugcard]`

```rust
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

That's it! The macro generates all the FFI wrappers and registration code.

## How It Works

The `#[plugcard]` attribute macro transforms your function into a plugin method:

1. **Input struct** - Arguments are bundled into a generated struct with serde derives
2. **FFI wrapper** - An `extern "C"` function that deserializes input, calls your function, serializes output
3. **Registration** - A static `MethodSignature` is registered in a distributed slice

### Generated Code

For a function like:

```rust
#[plugcard]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

The macro generates:

```rust
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

## API Reference

### `MethodSignature`

```rust
pub struct MethodSignature {
    pub key: u64,           // Unique key from name + schemas
    pub name: &'static str, // Human-readable method name
    pub input_schema: &'static NamedType,
    pub output_schema: &'static NamedType,
    pub call: unsafe extern "C" fn(*mut MethodCallData),
}
```

### `MethodCallData`

The FFI boundary structure:

```rust
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

### `MethodCallResult`

```rust
#[repr(C)]
pub enum MethodCallResult {
    Success,
    DeserializeError,
    SerializeError,
    MethodError,
    UnknownMethod,
}
```

### Introspection Functions

```rust
// Find a method by its computed key
pub fn find_method(key: u64) -> Option<&'static MethodSignature>

// List all registered methods
pub fn list_methods() -> &'static [MethodSignature]

// Dispatch a call by key
pub unsafe fn dispatch(data: *mut MethodCallData)
```

## Calling a Plugin Method

```rust
use plugcard::{list_methods, MethodCallData, MethodCallResult};

// Find the method
let methods = list_methods();
let add = methods.iter().find(|m| m.name == "add").unwrap();

// Prepare input (must match the generated input struct layout)
#[derive(Serialize)]
struct Input { a: i32, b: i32 }
let input_bytes = postcard::to_allocvec(&Input { a: 2, b: 3 }).unwrap();

// Prepare output buffer
let mut output_buf = [0u8; 256];
let mut data = MethodCallData {
    key: add.key,
    input_ptr: input_bytes.as_ptr(),
    input_len: input_bytes.len(),
    output_ptr: output_buf.as_mut_ptr(),
    output_cap: output_buf.len(),
    output_len: 0,
    result: MethodCallResult::default(),
};

// Call
unsafe { (add.call)(&mut data) };

// Check result
assert_eq!(data.result, MethodCallResult::Success);
let result: i32 = postcard::from_bytes(&output_buf[..data.output_len]).unwrap();
assert_eq!(result, 5);
```

## Method Keys

Method keys are computed at compile time using FNV-1a hash of:
- Method name
- Input schema name
- Output schema name

This ensures type-safe dispatch: if schemas change, keys change.

## Crate Structure

- **plugcard** - Core types and runtime
- **plugcard-macros** - The `#[plugcard]` proc macro (uses [unsynn](https://docs.rs/unsynn) for parsing)
- **dodeca-baseline** - Example plugin with test functions

## Why Plugins?

The primary motivation is **link speed**. Rust's incremental compilation is fast, but linking a large binary with many dependencies is slow. By moving functionality into dynamic libraries:

- The main `dodeca` binary stays small and links fast
- Plugins compile and link independently
- Changing a plugin doesn't require relinking the main binary
- Heavy dependencies (image processing, font subsetting, HTTP) live in plugins

This dramatically improves iteration speed during development.

## Future Architecture

The goal is to move most heavy dependencies into plugins:

```
┌─────────────────────────────────────────────────────────────┐
│                     Core dodeca                             │
│  ┌─────────┐ ┌─────────┐ ┌──────────┐ ┌─────────────────┐   │
│  │  Salsa  │ │Markdown │ │ Template │ │  Plugin Host    │   │
│  │(queries)│ │ Parser  │ │  Engine  │ │(JSON-RPC async) │   │
│  └─────────┘ └─────────┘ └──────────┘ └────────┬────────┘   │
└────────────────────────────────────────────────┼────────────┘
                                                 │
              ┌──────────────────────────────────┼──────────────────────────────────┐
              │                                  │                                  │
              ▼                                  ▼                                  ▼
┌─────────────────────────┐    ┌─────────────────────────┐    ┌─────────────────────────┐
│    http-server plugin   │    │   http-client plugin    │    │     search plugin       │
│        (axum)           │    │       (reqwest)         │    │  (replaces pagefind)    │
└─────────────────────────┘    └────────────┬────────────┘    └─────────────────────────┘
                                            │
                                            ▼
                               ┌─────────────────────────┐
                               │   link-checker plugin   │
                               │ (uses http-client msgs) │
                               └─────────────────────────┘

┌─────────────────────────┐    ┌─────────────────────────┐    ┌─────────────────────────┐
│  font-subsetting plugin │    │  image-processing plugin│    │      tui plugin         │
│      (fontcull)         │    │        (image)          │    │      (ratatui)          │
└─────────────────────────┘    └─────────────────────────┘    └─────────────────────────┘
```

### Async Plugin Support

Plugins can perform async operations via message-passing (JSON-RPC style):

```
Plugin                              Host
   │                                  │
   │── request { id: 1, ... } ───────▶│
   │                                  │ (host performs async I/O)
   │◀── response { id: 1, ... } ──────│
   │                                  │
```

Each plugin has its own runtime if needed. No shared tokio runtime complexity.

### Plugin Dependencies

Plugins can depend on each other through the message-passing system. For example, the link-checker plugin doesn't bundle its own HTTP client—it sends requests that get routed to the http-client plugin.

This keeps each plugin focused and avoids duplicating dependencies across plugins.
