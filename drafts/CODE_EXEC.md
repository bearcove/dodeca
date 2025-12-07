# Code Execution Improvements

## Current Problems

1. **Sequential execution** - Each code sample runs one at a time
2. **No caching** - `cache_dir` config exists but is unused; every build re-runs everything
3. **Fresh cargo project per sample** - Creates `/tmp/dodeca_sample_{pid}` each time
4. **No shared compilation** - Each sample compiles deps from scratch
5. **`--release` mode** - Unnecessary for doc samples, adds compile time
6. **Cargo output hidden** - `Stdio::piped()` captures but doesn't display during build

## Proposed Solutions

### 1. Dependency Pre-compilation

Before running any samples:
1. Create a single "deps" project in `.cache/code-execution/deps/`
2. Contains only `[dependencies]` from KDL config
3. Run `cargo build` once to compile all deps
4. Reuse this `target/` directory for all samples

```
.cache/code-execution/
  deps/
    Cargo.toml      # Just deps, no src
    Cargo.lock      # Locked versions
    target/         # Compiled deps, shared by all samples
```

Samples then use `CARGO_TARGET_DIR=../.cache/code-execution/deps/target`

### 2. Content-Addressable Caching

Cache key = hash of:
- Code content (after prepare_rust_code transforms)
- Dependencies (from config)
- Language config (command, args, etc.)

Cache structure:
```
.cache/code-execution/
  results/
    {hash}.json     # ExecutionResult serialized
```

On cache hit: skip execution, return cached result
On cache miss: execute, store result

### 3. Parallel Execution

Use rayon or tokio for parallel sample execution:
```rust
samples.par_iter().map(|sample| execute_single_sample(sample, config)).collect()
```

With shared deps target dir, cargo handles locking internally.

### 4. Streaming Output

Options:
- **During build**: Print cargo output as it happens (like `cargo build` does)
- **After execution**: Print summary with stdout/stderr for failures
- **Verbose mode**: Always show all output

Could use `inherit` for stdout/stderr in dev mode, `piped` in CI.

### 5. Incremental Dependency Updates

When deps change in KDL config:
1. Detect changes via hash of deps list
2. Update `deps/Cargo.toml`
3. Run `cargo build` to update deps
4. Invalidate all cached results (deps changed = results invalid)

### 6. Skip `--release`

Debug mode is fine for doc samples. Faster compile, good enough perf.

```rust
args: vec!["run".to_string(), "--quiet".to_string()],
```

### 7. Better Error Display

Show:
- Which file/line the sample came from
- The actual code that failed
- Compiler errors with ANSI colors preserved
- Runtime panics with backtraces

## Implementation Order

1. Remove `--release` (trivial win)
2. Show cargo output during execution
3. Add content-addressable caching
4. Add dependency pre-compilation
5. Add parallel execution
6. Incremental dep updates

## Open Questions

- Should cache survive `cargo clean`? (probably yes, it's in `.cache/`)
- Cache invalidation on rustc version change?
- Per-sample timeout vs global timeout?
- How to handle samples that intentionally fail? (already have `expected_errors`)
