+++
title = "Failing Code Sample"
+++

# Failing Code Sample

This page has a code sample that fails to compile.

```rust,test
fn main() {
    // This will cause a compilation error
    let x: i32 = "not a number";
    println!("{}", x);
}
```

The above code should fail with a type mismatch error.
