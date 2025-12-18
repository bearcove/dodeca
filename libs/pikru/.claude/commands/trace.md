# Tracing C vs Rust Code

When debugging mismatches between C pikchr and Rust pikru, **use data, not guesses**.

## Add Debug Tracing

**C code** - Add `DBG()` macro calls:
```c
DBG("[C cylinderFit] h=%g rad=%g sw=%g result=%g\n", h, pObj->rad, pObj->sw, pObj->h);
```

**Rust code** - Add `tracing::debug!()` calls:
```rust
tracing::debug!(
    h = h.raw(),
    rad = rad.raw(),
    sw = sw.raw(),
    result = result.raw(),
    "cylinderFit"
);
```

## Run with Tracing Enabled

```bash
# Rust with tracing
RUST_LOG=debug cargo test test78 -- --nocapture 2>&1 | grep "cylinderFit"

# C with DBG output (if enabled)
./pikchr --debug test78.pikchr
```

## Tag Rust Functions with C References

All ported functions MUST have a `cref` comment linking to the C source:

```rust
// cref: cylinderFit (pikchr.c:3976)
fn cylinder_fit(ctx: &RenderContext, fit_h: Inches, rad: Inches, sw: Inches) -> Inches {
    // ...
}
```

## The Process

1. Identify the mismatch in SVG output (coordinates, dimensions, paths)
2. Find the C function that computes those values
3. Add DBG() to C code to see actual values
4. Add tracing::debug!() to equivalent Rust code
5. Compare the traces side-by-side
6. Fix the Rust code based on **observed data differences**

This approach fixes tests based on **data** and helps understand how both codebases flow, instead of guessing at what might be wrong.

## IMPORTANT: Leave Traces in Place!

**DO NOT remove debug traces after fixing a bug!**

- Keep `DBG()` calls in C code for future debugging
- Keep `tracing::debug!()` calls in Rust code
- The C implementation is not used in production - traces are helpful for the next bug
- Rust tracing is compile-time controlled via `RUST_LOG` environment variable
- Having traces in place makes the next investigation faster and easier
