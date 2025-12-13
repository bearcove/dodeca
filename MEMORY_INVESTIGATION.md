# Memory Investigation Handoff - 2025-12-13

## Summary

Investigation into memory exhaustion issues with dodeca. Found and fixed multiple issues, but uncovered additional problems requiring further work.

## Issues Found & Fixed ✅

### 1. Plugin Standardization (Commits: 456da42, 5fe392e)

**Problem**: All 16 plugin modules had duplicate boilerplate code for SHM setup, argument parsing, and error handling.

**Solution**:
- Created standardized `dodeca_plugin_runtime` crate
- All plugins now use `plugin_service!` and `run_plugin!` macros
- Removed ~1,650+ lines of duplicate code
- All plugins now fail fast with clear errors when SHM files don't exist

**Files Changed**:
- All 16 plugin `main.rs` files drastically simplified
- `crates/dodeca-plugin-runtime/src/lib.rs` created with macros
- Corrected misconception: mod-tui is NOT a "reverse plugin" - it's a regular plugin

### 2. SHM Slot Exhaustion (Commits: ac62ee1, 7a83150)

**Problem**: `ddc serve` was constantly hitting "no free slots available" errors, causing RPC call failures.

**Root Cause**: SHM transport configured with only 128 slots (8MB total), insufficient for concurrent workload.

**Solution**:
- Increased `slot_count`: 128 → 512 (4x, now 32MB total)
- Increased `ring_capacity`: 256 → 1024 (4x)
- Unified all SHM configs to reference `dodeca_plugin_runtime::SHM_CONFIG`
- Previously duplicated across `plugins.rs`, `tui_host.rs`, `plugin_server.rs`

**Result**: Zero slot allocation failures in testing.

### 3. Memory Safety Issue Reported

**Issue**: https://github.com/facet-rs/facet/issues/1285

**Problem**: `facet_postcard` lacks memory limits during deserialization. A corrupt cache file can claim "I'm a Vec with 7TB of elements" and the deserializer will attempt to allocate that much memory without validation.

**Evidence**: Corrupt cache file in `docs/.cache/` caused OOM kills (exit code 137) when `fs::read()` tried to read it as a directory, then deserialization attempted massive allocations.

**Workaround**: Run `ddc clean` to clear corrupt cache.

**Permanent Fix**: Needs implementation in facet-rs/facet (add memory limits, validate sizes against buffer length, incremental allocation).

## Issues Remaining ❌

### 1. CRITICAL: `block_in_place` and `block_on` Forbidden

**Location**: `crates/dodeca/src/plugins.rs:946`

**Current Code**:
```rust
pub fn highlight_code_rapace(code: &str, language: &str) -> Option<HighlightResult> {
    let client = syntax_highlight_client()?;
    let code = code.to_string();
    let language = language.to_string();
    match tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(client.highlight_code(code, language))
    }) {
        Ok(result) => Some(result),
        Err(e) => {
            warn!("syntax highlight service call failed: {}", e);
            None
        }
    }
}
```

**Problem**:
- `block_in_place` requires multi-threaded runtime
- But we need single-threaded runtime for plugins (confirmed in commit a464ad6)
- **block_on is FORBIDDEN**

**Why It Exists**:
- Called from `queries.rs:1460` → `highlight_code_block()`
- Which is called from markdown processing loop at `queries.rs:1375`
- This runs inside facet queries (synchronous context)
- But needs to make async RPC call to arborium plugin

**Call Chain**:
```
facet query (sync)
  → markdown_to_html() (sync)
    → Event::End(CodeBlock) handler
      → highlight_code_block() (sync)
        → plugins::highlight_code() (sync - SHOULD BE ASYNC)
          → client.highlight_code() (async RPC call)
```

**TODO**:
2. Determine if the caller can be made async
3. If not, consider alternative architectures:
   - Pre-compute all syntax highlighting before entering sync context
   - Use message passing / channels instead of direct async calls
   - Restructure query system to support async operations

**Action**: Add clippy lint to forbid `block_on` usage.

### 2. Pending RPC Call Queue Exhaustion

**Symptom**:
```
too many pending RPC calls; refusing new call pending_len=8192 max_pending=8192
```

**Location**: Hundreds of these warnings during `ddc build`

**Problem**: RPC calls are piling up faster than they can be processed. The pending call queue has a hard limit of 8192.

**Possible Causes**:
1. **The blocking code**: `block_in_place` might be preventing async processing
2. **No backpressure**: Queries might be spawning RPC calls without waiting for results
3. **Slow plugin responses**: Single-threaded plugins might be bottlenecked
4. **Missing await points**: Async code not properly yielding

**TODO**:
1. Fix the `block_in_place` issue first (likely root cause)
2. Add metrics to track RPC call latency
3. Consider adding backpressure mechanisms
4. Profile plugin performance

### 3. Plugins Use Single-Threaded Runtime (By Design)

**Current State**: All plugins correctly use `#[tokio::main(flavor = "current_thread")]`

**Why**: Prevents excessive thread spawning (see commit a464ad6 - was spawning 96 threads per plugin on 96-core machine, causing kernel lock contention)

**Confirmed Working**: This is the correct configuration for plugins.

**Do NOT Change**: Plugins should stay single-threaded.

## Architecture Notes

### SHM Configuration (All plugins must match)

```rust
// crates/dodeca-plugin-runtime/src/lib.rs
pub const SHM_CONFIG: ShmSessionConfig = ShmSessionConfig {
    ring_capacity: 1024, // 1024 descriptors in flight
    slot_size: 65536,    // 64KB per slot
    slot_count: 512,     // 512 slots = 32MB total
};
```

All host-side code now references this constant:
- `crates/dodeca/src/plugins.rs`: `PLUGIN_SHM_CONFIG`
- `crates/dodeca/src/tui_host.rs`: `SHM_CONFIG`
- `crates/dodeca/src/plugin_server.rs`: `SHM_CONFIG`

### Plugin Runtime Architecture

Plugins use standardized macros:

```rust
dodeca_plugin_runtime::plugin_service!(
    ServerType<ImplType>,
    ImplType
);

dodeca_plugin_runtime::run_plugin!(ImplType);
```

This handles:
- Argument parsing (`--shm-path=...`)
- SHM file existence check (waits 5 seconds, then fails with clear error)
- Transport creation
- Session setup
- Tracing initialization
- Dispatcher registration
- Main loop

## Testing Commands

```bash
# Build with memory protection (2GB limit)
systemd-run --user --scope -p MemoryMax=2G cargo build --bin ddc

# Clean cache (required if corrupt)
systemd-run --user --scope -p MemoryMax=2G ./target/debug/ddc clean

# Test build
systemd-run --user --scope -p MemoryMax=2G ./target/debug/ddc build docs

# Test serve (currently broken due to block_in_place)
systemd-run --user --scope -p MemoryMax=2G ./target/debug/ddc serve docs

# Run with debug logging
systemd-run --user --scope -p MemoryMax=2G \
    env RUST_LOG=debug,rapace_transport_shm=debug \
    ./target/debug/ddc build docs
```

## Next Steps (Priority Order)

1. **CRITICAL**: Remove `block_in_place` / `block_on` usage in `highlight_code_rapace`
   - Investigate call sites
   - Make caller async if possible
   - Add clippy lint to prevent future usage

2. **HIGH**: Fix pending RPC call queue exhaustion
   - Likely caused by #1
   - Add backpressure if needed

3. **MEDIUM**: Monitor facet-rs/facet#1285 for memory limit implementation
   - Once available, add deserialization limits to cache loading

4. **LOW**: Add validation to `ContentStore::open()` in `cas.rs`
   - Check `path.is_file()` before attempting to read
   - Handle deserialization errors gracefully

## References

- Previous investigation: commit a464ad6 (thread spawning issue)
- facet memory issue: https://github.com/facet-rs/facet/issues/1285
- Perf wrapper: `.config/perf-wrapper.sh` (uses frame pointers, not DWARF)
- Nextest config: `.config/nextest.toml` (heaptrack, strace, perf profiles)

## Branch

All changes are on branch: `memory-mystery`

Latest commits:
- `7a83150` - Add dodeca-plugin-runtime dependency
- `ac62ee1` - Increase SHM capacity
- `5fe392e` - Migrate mod-tui
- `456da42` - Standardize all plugins
