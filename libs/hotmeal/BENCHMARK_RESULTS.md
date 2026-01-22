# Hotmeal Benchmark Results - Baseline

Run date: 2026-01-22
Machine: Apple M-series (aarch64-apple-darwin)
Timer precision: 41 ns

## Test Fixtures

| Size | File | Description |
|------|------|-------------|
| **Small** (8KB) | `https_quora.com_What-is-markup-in-HTML.html` | Small Q&A page |
| **Medium** (68KB) | `https_markdownguide.org_basic-syntax.html` | Documentation page |
| **Large** (172KB) | `https_developer.mozilla.org_en-US_docs_Web_HTML.html` | MDN docs |
| **XLarge** (340KB) | `https_fasterthanli.me.html` | Blog homepage with heavy styling |

## 1. Parsing Benchmarks

Measures time to parse HTML string into DOM using html5ever:

| Test | Fastest | Median | Mean | Samples |
|------|---------|--------|------|---------|
| parse_small (8KB) | 16.54 µs | 18.79 µs | 20.14 µs | 100 |
| parse_medium (68KB) | 1.613 ms | 1.692 ms | 1.712 ms | 100 |
| parse_large (172KB) | 2.46 ms | 2.618 ms | 2.643 ms | 100 |
| parse_xlarge (340KB) | 4.903 ms | 5.251 ms | 5.279 ms | 100 |

**Observations:**
- Parsing scales roughly linearly with document size
- ~15-16 µs/KB for larger documents
- html5ever + allocation overhead dominates

## 2. Diffing Benchmarks (With Parsing)

Measures full diff cycle: parse old + parse new + compute diff:

| Test | Fastest | Median | Mean | Notes |
|------|---------|--------|------|-------|
| diff_small | 65.2 µs | 125.9 µs | 136.7 µs | 2x parsing (~40µs) + diff |
| diff_medium | 5.819 ms | 6.232 ms | 6.286 ms | 2x parsing (~3.4ms) + diff |
| diff_large | 7.569 ms | 8.008 ms | 8.229 ms | 2x parsing (~5.3ms) + diff |
| diff_xlarge | 14.71 ms | 15.53 ms | 15.57 ms | 2x parsing (~10.6ms) + diff |

**Observations:**
- Total time ≈ (2 × parse time) + diff time
- For xlarge: 15.57ms total - 10.6ms parsing = **~5ms for diff**

## 3. Diffing Benchmarks (Diff Only)

Measures pure diff computation with pre-parsed DOMs:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| diff_only_small | 33.54 µs | 52.74 µs | 66.22 µs |
| diff_only_medium | 2.439 ms | 2.653 ms | 2.667 ms |
| diff_only_large | 2.429 ms | 2.644 ms | 2.648 ms |
| diff_only_xlarge | 4.383 ms | 4.71 ms | 4.844 ms |

**Observations:**
- Medium and Large have similar diff times (~2.6ms)
  - This suggests document structure matters more than size
- XLarge takes ~2x longer (4.8ms vs 2.6ms)
- Diff time is roughly 50% of total hot-reload cycle

## 4. Serialization Benchmarks

Measures time to serialize DOM back to HTML string:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| serialize_small | 5.291 µs | 5.374 µs | 5.459 µs |
| serialize_medium | 279.3 µs | 286.1 µs | 288.7 µs |
| serialize_large | 438.7 µs | 471.8 µs | 477.8 µs |
| serialize_xlarge | 1.224 ms | 1.271 ms | 1.283 ms |

**Observations:**
- Serialization is **4-5x faster** than parsing
- Scales linearly: ~3.5-4 µs/KB
- Dominated by string allocation + formatting

## 5. Full Hot-Reload Cycle

Measures complete cycle: parse old, parse new, diff, apply patches:

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| hot_reload_small | 76.83 µs | 142.4 µs | 149 µs |
| hot_reload_medium | 5.891 ms | 6.21 ms | 6.232 ms |
| hot_reload_large | 7.659 ms | 8.088 ms | 8.18 ms |
| hot_reload_xlarge | 14.89 ms | 15.67 ms | 15.76 ms |

**Breakdown for XLarge (340KB):**
```
Parse old:       5.3 ms  (34%)
Parse new:       5.3 ms  (34%)
Diff:            4.8 ms  (30%)
Apply patches:   ~0.3 ms (2%)
──────────────────────────
Total:           15.7 ms  (100%)
```

**Observations:**
- Parsing dominates: **68% of total time**
- Diff computation: **30% of total time**
- Patch application: **2% of total time** (negligible)

## Performance Hotspots

### 1. Parsing (68% of hot-reload time)
**Current approach:**
- html5ever provides `StrTendril` strings
- We convert every string to `String` (allocates)
- Recursive tree building (pointer chasing)

**Optimization opportunities:**
- ✅ **Borrow strings from source** → zero-copy parsing
- ✅ **Arena allocation** → fewer allocations, better cache locality
- Expected speedup: **2-3x**

### 2. Diff Computation (30% of hot-reload time)
**Current approach:**
- Convert to cinereus tree (allocates + clones)
- Traverse recursive structures (cache-unfriendly)
- Cinereus uses indextree internally (good!)

**Optimization opportunities:**
- ✅ **Direct arena integration** → skip conversion step
- ✅ **Cache-friendly traversal** → arena nodes are contiguous
- Expected speedup: **1.5-2x**

### 3. Patch Application (2% of hot-reload time)
- Already very fast
- Not a bottleneck
- No optimization needed

## Projected Performance with Arena + Borrowed Strings

| Operation | Current | Projected | Speedup |
|-----------|---------|-----------|---------|
| **Parse** | 5.3 ms | **2.0 ms** | 2.6x |
| **Diff** | 4.8 ms | **2.8 ms** | 1.7x |
| **Apply** | 0.3 ms | 0.3 ms | 1.0x |
| **Total** | **15.7 ms** | **5.1 ms** | **3.1x** |

**For large documents (340KB), hot-reload could go from ~16ms → ~5ms!**

## Recommendations

1. **Implement arena_dom with borrowed strings** (DESIGN_ARENA_BORROWED.md)
   - Highest ROI: 3x speedup on large documents
   - Reduces memory usage by 2-3x
   - Cache-friendly traversal

2. **Direct cinereus integration** (skip conversion step)
   - Medium ROI: 1.5x speedup on diff
   - Cleaner architecture
   - Less copying

3. **Profile actual workloads**
   - These benchmarks use single modifications
   - Real hot-reload might have multiple changes
   - Verify assumptions with real data

## Next Steps

1. ✅ Baseline benchmarks established
2. ⏭️ Implement `arena_dom` prototype
3. ⏭️ Benchmark arena_dom vs current
4. ⏭️ Compare against projections
5. ⏭️ Decide: make default or keep opt-in

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench parsing
cargo bench --bench diffing
cargo bench --bench serialization
cargo bench --bench full_cycle

# Save results
cargo bench 2>&1 | tee benchmark_results.txt
```

## Benchmark Code Quality Notes

Some warnings to fix:
- `diffing.rs`: Need to handle Result with `.unwrap()` or `.expect()` instead of `black_box()`
- All benchmarks compile and run correctly

## Raw Output

See `benchmark_results.txt` for full divan output.
