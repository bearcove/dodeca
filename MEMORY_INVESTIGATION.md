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

## Issues Partially Fixed ⚠️

### 1. `block_in_place` and `block_on` Removal (Ongoing)

**Status**: Major progress made. Core RPC-intensive functions now properly async.

#### Fixed ✅

1. **`highlight_code` in plugins.rs** - Made fully async
   - Removed `block_in_place` / `block_on` wrapper
   - Updated `render_markdown` in queries.rs to use two-pass approach:
     - First pass: collect code blocks synchronously during markdown parsing
     - Second pass: highlight all code blocks in parallel using async
   - Avoids Send issues with markdown parser state

2. **URL rewriting in url_rewrite.rs** - Replaced lol_html with html5ever
   - lol_html uses sync callbacks that couldn't await
   - html5ever allows full DOM manipulation then serialization
   - `rewrite_urls_in_html`, `rewrite_urls_in_css`, `rewrite_string_literals_in_js` now async
   - Two-pass approach: parse/collect sync (drops !Send RcDom), then process async

3. **Minification in svg.rs** - Made fully async
   - `minify_html` and `optimize_svg` now async
   - Updated callers in queries.rs

4. **Clippy lint added** - See `clippy.toml`
   ```toml
   { path = "tokio::runtime::Handle::block_on", reason = "Avoid blocking on async - make function async instead" },
   { path = "tokio::runtime::Runtime::block_on", reason = "Avoid blocking on async (except at entry point) - make function async instead" },
   { path = "tokio::task::block_in_place", reason = "Avoid block_in_place - make function async instead" },
   ```

5. **`link_checker.rs`** - Made fully async
   - `check_external_links` was already async but used `block_on` internally
   - Simply replaced `block_on` with `.await`

#### Remaining ❌

Clippy now warns on remaining violations (23 total):

| File | Count | Notes |
|------|-------|-------|
| serve.rs | 20 | RPC responders for mod-http (sync methods calling async queries) |
| main.rs | 2 | Entry points (acceptable) |
| search.rs | 1 | CPU-intensive, runs in separate thread (intentional) |

**serve.rs** contains methods that respond to RPC requests from mod-http (which uses axum). These methods are sync but need to call async database queries, hence the `block_on`. Making these async would require changes to how the RPC server handles requests.

**search.rs** is intentional - pagefind index building is CPU-intensive and runs in a dedicated thread via `std::thread::spawn`. The `block_on` is needed to access the tokio runtime from that thread.

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

1. ✅ **DONE**: Remove `block_in_place` / `block_on` from core query functions
   - `highlight_code` - DONE
   - `rewrite_urls_in_html/css/js` - DONE (replaced lol_html with html5ever)
   - `minify_html` / `optimize_svg` - DONE
   - `image.rs` functions - DONE (9 calls removed)
   - `link_checker.rs` - DONE (1 call removed)
   - Clippy lint added to prevent future usage

2. **MEDIUM**: Fix remaining `block_on` calls in serve.rs (20 calls)
   - These are actix-web handlers - would require architectural changes
   - Options: migrate to async web framework, or use `spawn_blocking`
   - Lower priority since dev server is less critical than build path

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
- (pending) - Make svg.rs functions async, add clippy lint
- (pending) - Replace lol_html with html5ever for async URL rewriting
- (pending) - Make highlight_code async with two-pass approach
- `88ebb09` - Add call chain analysis for block_in_place usage
- `6e5ccf7` - Add memory investigation handoff
- `7a83150` - Add dodeca-plugin-runtime dependency
- `ac62ee1` - Increase SHM capacity
- `5fe392e` - Migrate mod-tui
- `456da42` - Standardize all plugins
