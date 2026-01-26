# Monomorphization Analysis for dodeca

Analysis performed 2026-01-26 to identify and reduce LLVM IR bloat from excessive monomorphization.

## Methodology

### Profiling tools

```bash
# Self-profiling (requires nightly)
cargo +nightly clean -p dodeca
RUSTFLAGS="-Zself-profile=/tmp/profile" cargo +nightly build -p dodeca --bin ddc
summarize summarize /tmp/profile/ddc-*
cd /tmp/profile && crox ddc-*

# LLVM lines analysis (primary tool used)
cargo llvm-lines -p dodeca --bin ddc 2>&1 | head -100

# Sum by crate
cargo llvm-lines -p dodeca --bin ddc 2>&1 | awk '
/^[[:space:]]+[0-9]+/ {
  gsub(/[(),]/, "", $1);
  lines = $1 + 0;
  func = $0;
  if (match(func, /[a-z_]+::/)) {
    crate = substr(func, RSTART, RLENGTH-2);
    sums[crate] += lines;
  }
}
END {
  for (c in sums) print sums[c], c;
}' | sort -rn | head -20

# Filter by crate
cargo llvm-lines -p dodeca --bin ddc 2>&1 | grep "facet_format::" | head -30
```

### Key metrics

- **LLVM IR lines**: Total lines of LLVM intermediate representation generated
- **Copies**: Number of monomorphized instances of a function
- **Threshold**: >30k monomorphization instances is considered excessive

## Baseline (before fixes)

| Crate | LLVM IR Lines | Notes |
|-------|---------------|-------|
| tokio | 510,491 | 262 copies of task runtime per async task type |
| core | 481,540 | Iterator combinators, Option/Result - unavoidable |
| facet_format | 384,682 | 6 copies (one per format: yaml, json, toml, postcard, etc.) |
| picante | 377,511 | 60 copies per query type (K, V pairs) |
| alloc | 351,246 | Box, Vec, Arc - unavoidable |
| ddc | 318,411 | Main application code |
| roam_session | 101,266 | 26 copies per RPC method type |
| **Total** | **~3,000,000** | |

## Issues Filed & Results

### 1. facet-rs/facet#1924 - facet-format deserializer

**Problem**: `FormatDeserializer<'input, BORROW, P>` generic over parser type `P`, but most methods only use `P` for error type.

**Example**: `set_scalar` was 31,704 lines × 6 copies = ~190k lines, but the actual logic didn't depend on `P`.

**Fix (PR #1925)**: Factor out non-generic inner functions:
```rust
// Non-generic inner - single copy
fn set_scalar_inner<'input, const BORROW: bool>(...) -> Result<..., InnerError>

// Generic wrapper - just converts errors
fn set_scalar(&mut self, ...) -> Result<..., DeserializeError<P::Error>> {
    set_scalar_inner(...).map_err(...)
}
```

**Result**:
- `set_scalar`: 31k × 6 → 5.5k × 1 + 306 × 6 wrappers
- facet_format total: 385k → 346k (39k saved, 10%)

**Remaining opportunity**: Same pattern applies to `deserialize_enum_*`, `deserialize_struct_*`, etc. (~200k more potential savings)

### 2. bearcove/roam#60 - Client-side RPC calls

**Problem**: Generated client code monomorphizes `Caller::call<Args>` and `decode_response<Ok, Err>` for each RPC method.

**Top offenders**:
- `call_with_metadata::{{closure}}`: 43,056 lines × 26 copies
- `call_with_metadata::{{closure}}::{{closure}}`: 30,758 lines × 156 copies

**Fix (PR #61)**: Use reflection-based serialization:
```rust
// New non-generic function using Shape for serialization
async fn call_with_metadata_by_shape(&self, method_id: u64, args: &dyn Reflect, shape: &Shape) -> Result<...>
```

**Result**:
- roam_session: 101k → 25k (76k saved, 75%)
- Also reduced tokio bloat since roam spawns were optimized

### 3. bearcove/picante#46 - Incremental computation

**Problem**: `DerivedIngredient<DB, K, V>` methods monomorphized for each query type.

**Top offenders**:
- `scope_if_needed::{{closure}}`: 31,570 lines × 110 copies
- `restore_runtime_state::{{closure}}`: 30,840 lines × 60 copies
- `get::{{closure}}`, `touch::{{closure}}`: ~20k × 50-60 copies each

**Fix (PRs #47, #48)**:
1. `scope_if_needed_boxed` - accepts `Pin<Box<dyn Future>>` instead of generic
2. `restore_runtime_state_inner` - non-generic helper on `DerivedCore`
3. Similar pattern for `touch`

**Result**:
- picante: 378k → 257k (121k saved, 32%)
- Also reduced tokio bloat from picante's spawns

### 4. bearcove/picante#50 - Further derived ingredient optimization

**Problem**: After PRs #47 and #48, `get::{{closure}}` and persistence callbacks remained top offenders.

**Fix (PR #50)**: Additional type erasure in derived ingredient.

**Result**:
- picante: 257k → 249k (8k saved, 3%)
- facet also updated in same cycle: 346k → 326k (20k saved)

### 5. facet-rs/facet#1928 - Coroutine-based deserializer

**Problem**: Large deserialize functions (`deserialize_struct_with_flatten`, `deserialize_enum_externally_tagged`, etc.) were still monomorphized 6× for each parser type.

**Fix (PR #1928)**: Rewrote deserializer to use stackful coroutines (corosensei), allowing type erasure of the parser type during deserialization.

**Result**:
- facet_format: 326k → 267k (59k saved, 18%)
- Added corosensei dependency: +12k lines
- Net savings: ~47k lines
- Total: 2.41M → 2.35M

### 6. bearcove/dodeca#218 - Move Vite to cell

**Status**: Open

**Rationale**: Vite dev server handling in main binary adds to monomorphization. Moving to a cell would isolate it.

### 7. facet-rs/facet#1936 - Remove ProbeStream GAT

**Problem**: `FormatParser` trait used a GAT `ProbeStream<'a>` for lookahead parsing. This required complex type machinery and prevented object safety.

**Fix (PR #1936)**: Replace GAT with simple `save()`/`restore()` methods using clone-based state management:
```rust
// Old: GAT-based probing
type ProbeStream<'a>: FormatParser<...>;
fn build_probe(&mut self) -> Self::ProbeStream<'_>;

// New: Clone-based save/restore
fn save(&mut self) -> SavePoint;
fn restore(&mut self, save_point: SavePoint);
```

**Affected crates**: All format parsers (facet-json, facet-yaml, facet-toml, facet-postcard), plus downstream: figue, facet-styx.

**Result**:
- Total: 2.35M → 2.1M (~250k saved, 11%)
- Simplified parser implementations
- Better object-safety for future `dyn FormatParser` usage

### 8. facet-rs/facet#1939 - Implement dyn FormatParser

**Problem**: Each format (JSON, YAML, TOML) monomorphizes `FormatDeserializer` separately when used for runtime format selection.

**Fix (PR #1939)**: Implement `FormatParser` for `&mut dyn DynParser<'de>`:
```rust
// Single function handles all formats via dynamic dispatch
fn deserialize_value(parser: &mut dyn DynParser<'_>) -> Result<Value, DynDeserializeError> {
    let mut de = FormatDeserializer::new(parser);
    de.deserialize()
}
```

**Also**: Switched dodeca from `serde_yaml` to `facet_yaml` for data file parsing.

**Result**:
- Enables runtime format selection with single deserializer monomorphization
- facet_format: 267k → 282k (+15k for dyn dispatch infrastructure)
- tokio: 302k → 149k (-153k, unrelated improvement from dependency updates)
- Net total: 2.1M → 2.16M (slight increase from adding facet-yaml dependency)

## Final Results

| Crate | Before | After | Savings | % |
|-------|--------|-------|---------|---|
| tokio | 510k | 149k | 361k | 71% |
| picante | 378k | 242k | 136k | 36% |
| facet_format | 385k | 282k | 103k | 27% |
| roam_session | 101k | 25k | 76k | 75% |
| core | 481k | 382k | 99k | 21% |
| alloc | 351k | 308k | 43k | 12% |
| **Total** | **3.0M** | **2.16M** | **~840k** | **28%** |

*(Measurements after facet #1939 dyn FormatParser and switching to facet-yaml)*

## Remaining Opportunities

### bearcove/picante#49 - Further picante reduction (partially addressed by #50)

| Function | Lines × Copies | Status |
|----------|----------------|--------|
| `get::{{closure}}` | 17k × 50 | Still present |
| `TypedCompute::compute::{{closure}}` | 14k × 60 | Still present |
| `access_scoped_erased::{{closure}}` | 16k × 2 | New addition from PR #50 |
| Persistence callbacks | ~50k × 30-150 | Still present |

### facet-format additional functions

**Addressed by PR #1928** - Coroutine-based deserializer eliminated most of these:
- `deserialize_struct_with_flatten`: Gone (was 28k × 6)
- `deserialize_enum_externally_tagged`: Gone (was 21k × 6)
- `deserialize_enum_internally_tagged`: Gone (was 17k × 6)

Remaining facet_format hotspots (post-#1936):
| Function | Lines × Copies |
|----------|----------------|
| `deserialize_into` | 20k × 6 |

The ProbeStream removal simplified parser implementations and reduced GAT-related monomorphization overhead.

## Patterns Identified

### Pattern 1: Generic over error type only

When a function is generic over `P` but only uses `P::Error` in the return type:
- Factor out logic into non-generic inner function
- Thin generic wrapper converts error type

### Pattern 2: Async blocks in generic impl

Async blocks capture `self` which includes generic params, causing monomorphization even if the block doesn't use them:
- Box the future: `Box::pin(async { ... })`
- Or move logic to non-generic helper that returns `BoxFuture`

### Pattern 3: Serialization/deserialization

`facet_postcard::to_vec::<T>()` and `from_slice::<T>()` monomorphize for each type:
- Use reflection-based serialization when possible
- Consider type-erased intermediate representation

### Pattern 4: Task spawning

Each unique future type passed to `tokio::spawn` creates copies of task machinery:
- Box futures before spawning: `tokio::spawn(Box::pin(async { ... }))`
- Or consolidate similar tasks

### Pattern 5: GATs for lookahead/probing

GATs (Generic Associated Types) prevent object safety and add monomorphization overhead:
- Replace GAT-based probing with clone-based save/restore
- If the type is `Clone`, just clone state on save and restore by swapping back
- Enables future `dyn Trait` usage for additional type erasure

## Commands Reference

```bash
# Update dependencies
cargo update -p facet
cargo update -p picante
cargo update -p roam-session

# Clean specific crate before measuring
cargo clean -p <crate>

# Build and measure
cargo build -p dodeca --bin ddc
cargo llvm-lines -p dodeca --bin ddc 2>&1 | head -100

# Incremental build timing
touch crates/dodeca/src/main.rs
time cargo build -p dodeca --bin ddc
```
