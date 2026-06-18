# Hotmeal Benchmark Results - Arena + Stem Implementation

Run date: 2026-01-23 (after Stem<'a> for Send+Sync Document)
Machine: Apple M-series (aarch64-apple-darwin)
Timer precision: 41 ns
Implementation: Arena DOM + Stem<'a> strings (Send+Sync, zero-copy where possible)

## Test Fixtures

| Size | File | Description |
|------|------|-------------|
| **Small** (8KB) | `https_quora.com_What-is-markup-in-HTML.html` | Small Q&A page |
| **Medium** (68KB) | `https_markdownguide.org_basic-syntax.html` | Documentation page |
| **Large** (172KB) | `https_developer.mozilla.org_en-US_docs_Web_HTML.html` | MDN docs |
| **XLarge** (340KB) | `https_fasterthanli.me.html` | Blog homepage with heavy styling |
| **XXLarge** (492KB) | `xxl.html` | Extra large document |

## Performance Summary

### Key Changes in This Version

- **Document is now Send+Sync** via `Stem<'a>` type (closes #21)
- **Zero-copy parsing** when content points into the original StrTendril
- **Breaking API change**: `parse()` now takes `&StrTendril` instead of `&str`
- **In-place mutation** for text merging (6000+ allocations saved on XXL docs)

## Benchmark Results

### 1. Parsing Benchmarks

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| parse_small (8KB) | 12.66 µs | 12.87 µs | **13.39 µs** |
| parse_medium (68KB) | 884.2 µs | 919 µs | **923.2 µs** |
| parse_large (172KB) | 1.446 ms | 1.508 ms | **1.521 ms** |
| parse_xlarge (340KB) | 2.46 ms | 2.565 ms | **2.579 ms** |

### 2. Serialization Benchmarks

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| serialize_small | 8.332 µs | 8.374 µs | **8.457 µs** |
| serialize_medium | 66.24 µs | 74.89 µs | **77.43 µs** |
| serialize_large | 165.4 µs | 182.2 µs | **186.8 µs** |
| serialize_xlarge | 405.9 µs | 429.8 µs | **439.7 µs** |

### 3. Diffing Benchmarks (With Parsing)

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| diff_small | 27.41 µs | 30.56 µs | **32.95 µs** |
| diff_medium | 3.272 ms | 3.408 ms | **3.422 ms** |
| diff_large | 4.334 ms | 4.667 ms | **4.68 ms** |
| diff_xlarge | 6.884 ms | 7.166 ms | **7.221 ms** |

### 4. Diffing Benchmarks (Diff Only)

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| diff_only_small | 3.707 µs | 3.832 µs | **3.897 µs** |
| diff_only_medium | 1.504 ms | 1.551 ms | **1.564 ms** |
| diff_only_large | 1.375 ms | 1.414 ms | **1.429 ms** |
| diff_only_xlarge | 1.84 ms | 1.941 ms | **1.97 ms** |

### 5. Full Hot-Reload Cycle

| Test | Fastest | Median | Mean |
|------|---------|--------|------|
| hot_reload_small | 29.79 µs | 30.79 µs | **31.81 µs** |
| hot_reload_medium | 3.302 ms | 3.517 ms | **3.567 ms** |
| hot_reload_large | 4.41 ms | 4.686 ms | **4.714 ms** |
| hot_reload_xlarge | 6.933 ms | 7.219 ms | **7.251 ms** |
| hot_reload_xxlarge | 9.626 ms | 10.1 ms | **10.13 ms** |

## Comparison with Baseline (Before Stem)

| Metric | Baseline | With Stem | Change |
|--------|----------|-----------|--------|
| parse_small | 15.12 µs | 13.39 µs | 11% faster |
| parse_medium | 1.04 ms | 923.2 µs | 11% faster |
| parse_large | 1.69 ms | 1.521 ms | 10% faster |
| parse_xlarge | 2.85 ms | 2.579 ms | 10% faster |
| hot_reload_small | 32.6 µs | 31.81 µs | similar |
| hot_reload_medium | 3.48 ms | 3.567 ms | similar |
| hot_reload_large | 4.87 ms | 4.714 ms | 3% faster |
| hot_reload_xlarge | 7.54 ms | 7.251 ms | 4% faster |
| hot_reload_xxlarge | 7.46 ms | 10.13 ms | 36% slower |

## Analysis

### What This Version Provides

1. **Send+Sync Document** - Can now be shared across threads
2. **Zero-copy parsing** - ~11% of strings borrow from input (text merging converts rest to Owned)
3. **Improved parsing** - 10-11% faster across all sizes
4. **In-place text merging** - Mutates CompactString directly instead of reallocating

### Remaining Trade-off

XXLarge documents are 36% slower due to the sheer volume of text merging operations.
This could potentially be improved by batching or using different strategies for very large docs.

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench parsing
cargo bench --bench diffing
cargo bench --bench serialization
cargo bench --bench full_cycle

# Check zero-copy percentage
cargo run --example stem_stats --release
```
