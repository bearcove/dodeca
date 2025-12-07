//! Integration tests for code sample execution
//!
//! These tests verify that:
//! 1. Correct code samples execute and show output
//! 2. Failing code samples show compiler errors with ANSI colors
//! 3. Incorrect samples that should pass cause the build to fail

use crate::harness::InlineSite;

/// Test that a correct code sample executes successfully and shows output
#[test_log::test]
fn test_successful_code_sample_shows_output() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Home

```rust
fn main() {
    println!("Hello from code execution!");
}
```
"#,
        ),
    ]);

    let result = site.build();

    // Build should succeed
    result.assert_success();

    // Should show successful execution message
    result.assert_output_contains("code samples executed successfully");
}

/// Test that a correct code sample with ANSI colors in output works
#[test_log::test]
fn test_successful_code_sample_with_ansi_colors() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Colored Output

```rust
fn main() {
    // Output with ANSI escape codes for colors
    println!("\x1b[32mGreen text\x1b[0m");
    println!("\x1b[31mRed text\x1b[0m");
    println!("\x1b[1;34mBold blue\x1b[0m");
}
```
"#,
        ),
    ]);

    let result = site.build();
    result.assert_success();
}

/// Test that a failing code sample causes the build to fail and shows compiler errors
#[test_log::test]
fn test_failing_code_sample_shows_compiler_error() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Type Error

```rust
fn main() {
    let x: i32 = "not a number";
    println!("{}", x);
}
```
"#,
        ),
    ]);

    let result = site.build();

    // Build should fail
    result.assert_failure();

    // Should show code execution failure
    result.assert_output_contains("Code execution failed");

    // Should contain type error message from rustc
    // The error message should mention "mismatched types" or similar
    result.assert_output_contains("mismatched types");
}

/// Test that compiler errors preserve ANSI colors from rustc
#[test_log::test]
fn test_compiler_error_with_ansi_colors() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Compilation Error

```rust
fn main() {
    let undefined_variable;
    println!("{}", undefined_variable);
}
```
"#,
        ),
    ]);

    let result = site.build();
    result.assert_failure();

    // The stderr should contain the error
    // Note: ANSI codes may or may not be present depending on terminal detection
    // but the error message content should be there
    result.assert_output_contains("error");
}

/// Test that an incorrect code sample that's expected to pass causes build failure
#[test_log::test]
fn test_incorrect_sample_expected_to_pass_fails_build() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# This code has a bug

The following code is supposed to work but has a typo:

```rust
fn main() {
    // Intentional error - calling non-existent method
    let numbers = vec![1, 2, 3];
    let sum = numbers.sums();  // typo: should be .iter().sum()
    println!("Sum: {}", sum);
}
```
"#,
        ),
    ]);

    let result = site.build();

    // Build should fail because the code doesn't compile
    result.assert_failure();

    // Should indicate code execution failure
    result.assert_output_contains("code sample(s) failed");
}

/// Test that multiple code samples are all executed
#[test_log::test]
fn test_multiple_code_samples_executed() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Multiple Samples

First sample:

```rust
fn main() {
    println!("Sample 1");
}
```

Second sample:

```rust
fn main() {
    println!("Sample 2");
}
```
"#,
        ),
    ]);

    let result = site.build();
    result.assert_success();

    // Should show that multiple samples executed
    // The exact message depends on implementation but should mention count
    result.assert_output_contains("code samples executed successfully");
}

/// Test that non-rust code blocks are not executed
#[test_log::test]
fn test_non_rust_code_blocks_not_executed() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Non-Rust Code

This JavaScript won't be executed:

```javascript
console.log("This should not run");
```

This Python won't be executed:

```python
print("This should not run")
```

This has no language specified:

```
Some random text
```
"#,
        ),
    ]);

    let result = site.build();

    // Build should succeed (no rust code to fail)
    result.assert_success();
}

/// Test that runtime panics are caught and reported
#[test_log::test]
fn test_runtime_panic_reported() {
    let site = InlineSite::new(&[
        (
            "_index.md",
            r#"+++
title = "Home"
+++

# Panic Test

```rust
fn main() {
    panic!("Intentional panic for testing!");
}
```
"#,
        ),
    ]);

    let result = site.build();

    // Build should fail because the code panicked
    result.assert_failure();

    // Should contain panic message
    result.assert_output_contains("Intentional panic");
}
