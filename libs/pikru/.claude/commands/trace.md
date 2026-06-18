# Tracing C vs Rust Code

When debugging mismatches between C pikchr and Rust pikru, **use data, not guesses**.

## Directory Structure

```
pikru/
├── src/                        # Rust implementation
│   ├── render/mod.rs           # Main rendering logic
│   ├── render/shapes.rs        # Shape implementations
│   └── render/types.rs         # Type definitions
├── vendor/pikchr-c/            # C reference implementation
│   ├── pikchr.c                # Main source (with DBG macros)
│   ├── pikchr.y                # Parser grammar (lemon)
│   ├── pikchr                  # Release build (no debug output)
│   ├── pikchr_debug            # Debug build (DBG enabled)
│   ├── Makefile                # Build rules
│   └── tests/                  # Test .pikchr files
├── debug-svg/                  # Generated SVGs for comparison
│   ├── test23-c.svg            # C output (reference)
│   └── test23-rust.svg         # Rust output (actual)
└── tests/pikchr_tests.rs       # Rust test harness
```

## Building pikchr-c

```bash
cd vendor/pikchr-c

# Release build (DBG disabled)
make pikchr

# Debug build (DBG enabled) - outputs to stderr
cc -O0 -g -Wall -Wextra -DPIKCHR_SHELL -DPIKCHR_DEBUG pikchr.c -o pikchr_debug -lm
```

**Note:** `make clean` is disabled to preserve debug modifications.

## The DBG Macro

In `pikchr.c` (around line 195):

```c
#ifdef PIKCHR_DEBUG
#define DBG(...) fprintf(stderr, __VA_ARGS__)
#else
#define DBG(...) ((void)0)
#endif
```

Add traces like:
```c
DBG("[C pik_size_to_fit] bbox: sw=(%g,%g) ne=(%g,%g)\n",
    bbox.sw.x, bbox.sw.y, bbox.ne.x, bbox.ne.y);
```

Run with: `./pikchr_debug tests/test23.pikchr 2>&1 | grep "pik_size_to_fit"`

## Rust Tracing

Add `tracing::debug!()` calls (NOT `eprintln!`):

```rust
tracing::debug!(
    bbox_min_x = bbox_min_x,
    bbox_max_x = bbox_max_x,
    "pik_size_to_fit bbox"
);
```

Run with: `RUST_LOG=debug cargo test test23 -- --nocapture 2>&1 | grep "pik_size_to_fit"`

## Too Much Tracing Kills Tracing

**Don't hesitate to REMOVE traces if:**
- Output is too verbose / overwhelming
- Traces are truncated due to token limits
- They're no longer useful for the current investigation

Keep traces **targeted** - add them for the specific issue you're debugging. A wall of output is worse than no output. You can always re-add traces later.

## The Process

1. Identify the mismatch in SVG output
2. Find the C function that computes those values (use `grep -n` on pikchr.c)
3. Add targeted DBG() to C code
4. Add matching tracing::debug!() to Rust code
5. Compare the traces side-by-side
6. **Remove traces** that are no longer useful
7. Fix the Rust code based on **observed data differences**

## Tag Rust Functions with C References

All ported functions MUST have a `cref` comment:

```rust
// cref: cylinderFit (pikchr.c:3976)
fn cylinder_fit(...) -> Inches {
```

## Running Tests

**NEVER run full test suite** - it times out.

```bash
# Single test
cargo test test23 -- --nocapture

# With tracing
RUST_LOG=debug cargo test test23 -- --nocapture 2>&1 | grep "keyword"
```

## Comparing SVG Output

```bash
# Compare specific element
grep "3 leading" debug-svg/test23-c.svg debug-svg/test23-rust.svg

# Diff the SVGs
diff debug-svg/test23-c.svg debug-svg/test23-rust.svg | head -50
```
