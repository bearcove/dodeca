# Development Guide

## Fuzzing Workflow

Hotmeal uses cargo-fuzz (libFuzzer) to find bugs in the diff/patch implementation. The fuzzing workflow helps catch edge cases that are hard to find through traditional testing.

### Prerequisites

Fuzzing requires nightly Rust:

```bash
rustup install nightly
```

### Running the Fuzzer

Run the fuzzer for a specified time (e.g., 60 seconds):

```bash
cd hotmeal/fuzz
cargo +nightly fuzz run roundtrip -- -max_total_time=60
```

The fuzzer will:
- Generate random HTML structures
- Compute diffs between old and new HTML
- Apply patches to transform old → new
- Verify the result matches the expected output

When a crash is found, libFuzzer will:
- Print the failing input and assertion details
- Save the crash to `artifacts/roundtrip/crash-<hash>`
- Show the `Debug` representation of the input
- Provide commands to reproduce and minimize

### Reproducing a Crash

Once a crash is found, reproduce it with:

```bash
cargo +nightly fuzz run roundtrip artifacts/roundtrip/crash-<hash>
```

This runs only that specific input, showing the full panic output.

### Converting Crashes to Test Cases

1. **Extract the HTML from the fuzzer output:**
   - The fuzzer prints `Old:` and `New:` HTML strings
   - Or look at the `Debug` output showing the `FuzzInput` structure

2. **Add a test in `hotmeal/src/diff/tree.rs`:**

```rust
#[test]
fn test_fuzzer_<description>() {
    let old_html = r#"<html>...</html>"#;
    let new_html = r#"<html>...</html>"#;

    let patches = super::super::diff_html(old_html, new_html)
        .expect("diff failed");
    debug!("Patches: {:#?}", patches);

    let mut tree = super::super::apply::parse_html(old_html)
        .expect("parse old failed");
    super::super::apply::apply_patches(&mut tree, &patches)
        .expect("apply failed");

    let result = tree.to_html();
    let expected_tree = super::super::apply::parse_html(new_html)
        .expect("parse new failed");
    let expected = expected_tree.to_html();

    debug!("Result: {}", result);
    debug!("Expected: {}", expected);
    assert_eq!(result, expected, "HTML output should match");
}
```

3. **Run the test:**

```bash
cd hotmeal
cargo test test_fuzzer_<description>
```

### Debugging with Tracing

Hotmeal uses facet's tracing macros (`debug!` and `trace!`) for debugging. Enable them:

```bash
# Run tests with tracing enabled
RUST_LOG=hotmeal=debug cargo test test_fuzzer_<description> -- --nocapture

# For even more detail:
RUST_LOG=hotmeal=trace cargo test test_fuzzer_<description> -- --nocapture
```

**Logging levels:**
- `debug!` - High-level operations (e.g., "starting diff", "found 5 patches")
- `trace!` - Detailed step-by-step info (e.g., "checking node X", "inserting at position Y")

**Adding debug output:**

In test code, use `#[cfg(test)]` to add temporary debug printing:

```rust
#[cfg(test)]
shadow_tree.debug_print_tree("After Move");
```

This pretty-prints the shadow tree structure showing:
- Tree hierarchy with node IDs
- Detached nodes in slots
- Element tags and text content

### Minimizing Test Cases

Reduce a crashing input to its minimal form:

```bash
cargo +nightly fuzz tmin roundtrip artifacts/roundtrip/crash-<hash>
```

This finds the smallest input that still triggers the bug, making it easier to understand and debug.

### Complete Fuzzing Workflow

1. **Run fuzzer** until it finds a crash
2. **Reproduce** the crash to confirm it's real
3. **Minimize** the test case (optional but recommended)
4. **Create test** by copying HTML into `hotmeal/src/diff/tree.rs`
5. **Run test** to confirm it fails
6. **Enable tracing** to understand what's happening:
   ```bash
   RUST_LOG=hotmeal=debug cargo test test_fuzzer_foo -- --nocapture
   ```
7. **Add debug printing** in relevant code sections
8. **Identify root cause** from trace output
9. **Fix the bug** - could be in:
   - `hotmeal/src/diff/tree.rs` - patch generation
   - `hotmeal/src/diff/apply.rs` - patch application
   - `cinereus/src/` - tree diffing algorithm
10. **Verify fix** - test should pass
11. **Run full test suite** to ensure no regressions:
    ```bash
    cargo test
    ```
12. **Run fuzzer again** to find the next bug!

### Common Bug Patterns

**Detached node tracking:**
- Nodes in slots must be tracked correctly
- Children of detached nodes need relative paths
- Always check if a node or its ancestor is detached

**Path invalidation:**
- Paths become invalid after moves/inserts/deletes
- Use shadow tree to track current positions
- Operations must happen in the right order

**Displacement semantics:**
- When inserting at position, existing node moves to a slot
- The slot preserves the displaced node for later reuse
- Moves can reference slots via `NodeRef::Slot(n, path)`

**Simplification errors:**
- Child operations are dominated when parent is moved/inserted/deleted
- BUT only if they were parent-child in BOTH source and destination trees
- Check relationships carefully before dropping operations

### Tips

- Start fuzzing runs short (60s) to get quick feedback
- Once stable, run longer sessions overnight to find deeper bugs
- Keep the artifact history - old crashes might resurface
- Document root causes in test names and comments
- When stuck, add more `debug!` output before adding more code
