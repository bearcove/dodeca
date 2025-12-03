# The Unsafe Mystery: Plugin Method Names Corrupted

## RESOLVED

**The issue was caused by stale `.so` plugin files.** After rebuilding both plugins,
the issue went away completely. The WebP plugin loaded correctly, but the JXL plugin
had corrupted method names because it was built against an older version of `plugcard`
(before the facet-postcard migration in commit cad6498).

The fix is simple: **rebuild all plugins after modifying `plugcard` or its dependencies**.
Cargo doesn't automatically track ABI changes across cdylib boundaries.

---

## Original Investigation

## Summary

When loading plugins via `plugcard`, the `MethodSignature.name` field (a `&'static str`)
sometimes contains garbage data, causing a panic when attempting to format/print it.

## The Crash

```
byte index 3 is not a char boundary; it is inside 'W' (bytes 2..3) of `!W�.{...`
```

The panic occurs in `tracing-subscriber` when trying to debug-format a `Vec<&str>` of
method names from a loaded plugin.

## Key Finding

Debug output showed:

```
[DEBUG] method[0]: ptr=0x7b403a09c05e len=11
[DEBUG] method[0]: bytes=[100, 101, 99, 111, 100, 101, 95, 119, 101, 98, 112]
[DEBUG] method[0]: valid utf8 = "decode_webp"

[DEBUG] method[1]: ptr=0x7b403a13c038 len=135515782496424
[DEBUG] method[1]: bytes=[27, 193, 9, 58, 64, 123, ...]
[DEBUG] method[1]: INVALID UTF8: invalid utf-8 sequence of 1 bytes from index 1
```

**Method[0]** is correct: `"decode_webp"` with len=11.

**Method[1]** is corrupted:
- `len = 135515782496424` (135 TRILLION - obviously wrong!)
- In hex: `0x7B403A09C018` - this looks like a **pointer**, not a length!

## Root Cause Hypothesis

The `MethodSignature` struct layout is being read incorrectly across the dylib boundary.

```rust
pub struct MethodSignature {
    pub key: u64,                                    // 8 bytes
    pub name: &'static str,                          // 16 bytes (ptr + len)
    pub input_type_name: &'static str,               // 16 bytes
    pub output_type_name: &'static str,              // 16 bytes
    pub call: unsafe extern "C" fn(*mut MethodCallData), // 8 bytes
}
// Total: 64 bytes
```

A `&str` is a fat pointer: `(ptr: *const u8, len: usize)` = 16 bytes on 64-bit.

The corrupted length `0x7B403A09C018` being a pointer-like value suggests that when
reading method[1], the struct fields are offset incorrectly - we're reading what
should be the `input_type_name.ptr` as the `name.len`.

## What's NOT the Cause

1. **NOT facet-postcard** - The `MethodSignature` is never serialized/deserialized.
   It's read directly from plugin memory via `std::slice::from_raw_parts`.

2. **NOT invalid UTF-8 encoding** - The bytes themselves are fine; the problem is
   the `len` field contains garbage.

3. **NOT a use-after-free** - ASAN didn't detect any memory errors before the panic.

4. **NOT compiler version mismatch** - Host and plugin built with same rustc.

## What IS Suspicious

1. **linkme** - The distributed slice mechanism might not be correctly handling the
   struct across dylib boundaries.

2. **Struct alignment/padding** - Without `#[repr(C)]`, Rust can reorder fields.
   If the host and plugin have different ideas about field order, chaos ensues.

3. **The first method works, second doesn't** - This suggests an array stride issue.
   If the host thinks `MethodSignature` is 64 bytes but the plugin thinks it's 72
   (or vice versa), the second element would be read at the wrong offset.

## The Code Path

1. Plugin exports `__plugcard_methods_ptr()` and `__plugcard_methods_len()`
2. Host calls these via libloading
3. Host does `std::slice::from_raw_parts(ptr, len)` to create `&[MethodSignature]`
4. Host iterates and reads `.name` from each
5. Second element's `.name.len` is garbage → panic when formatting

## Files Involved

- `crates/plugcard/src/lib.rs` - `MethodSignature` struct definition
- `crates/plugcard/src/loader.rs` - Plugin loading, lines 30-41
- `crates/plugcard-macros/src/lib.rs` - Generates the static `MethodSignature` instances
- `src/plugins.rs` - Where the crash manifests

## Next Steps to Investigate

1. **Add `#[repr(C)]` to `MethodSignature`** - Ensure consistent layout

2. **Print struct sizes from both sides**:
   - Add `println!("sizeof MethodSignature = {}", std::mem::size_of::<MethodSignature>())`
     to both plugin and host

3. **Check linkme version compatibility** - Both host and plugin use linkme 0.3

4. **Inspect the actual bytes** - Dump the raw bytes at the pointer and verify
   they match what the plugin thinks it wrote

5. **Review all unsafe code in plugcard**:
   - `loader.rs:31` - `Library::new`
   - `loader.rs:35-37` - `library.get` for symbols
   - `loader.rs:41` - `slice::from_raw_parts`
   - `loader.rs:46` - Dereferencing dispatch function pointer

## Minimal Repro

A minimal repro exists at `/home/amos/bearcove/facet-postcard-repro/` with:
- `host/` - loads the plugin
- `plugin/` - exports methods via linkme

The minimal repro **works correctly** - it doesn't reproduce the bug. This suggests
the issue is specific to something in the full dodeca/plugcard setup that the minimal
repro doesn't capture.

## Environment

- Platform: Linux x86_64
- Rust: nightly (for ASAN)
- ASAN enabled for debugging
- Same compiler for host and plugin

---

## Resolution Details

After extensive debugging with:
- ASAN builds
- Debug output showing corrupted `len` fields (pointer values instead of lengths)
- Size checking code added to both host and plugins

The issue disappeared after adding a `debug_struct_size` function to the JXL plugin,
which forced a rebuild. Both plugins now show `host=64, plugin=64` bytes for
`MethodSignature` and load correctly.

**Root cause**: The JXL plugin `.so` file was stale, built before recent `plugcard`
changes. Cargo's dependency tracking doesn't extend across cdylib boundaries, so
changing `plugcard` doesn't automatically trigger a rebuild of plugins that depend on it.

**Lesson**: When developing plugin systems, always do a clean rebuild of all plugins
after changing the plugin infrastructure. Consider adding a build-time hash check or
version number to detect ABI mismatches at runtime.
