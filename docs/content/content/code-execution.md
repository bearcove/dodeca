+++
title = "Code Execution"
weight = 50
+++

dodeca can compile and run Rust code blocks during the build, verifying that your examples actually work.

## Usage

Add the `test` annotation to a Rust code block:

````markdown
```rust,test
let x = 2 + 2;
assert_eq!(x, 4);
```
````

The code is automatically wrapped in a `fn main() { ... }` block, compiled, and executed. If it fails, the build fails — your docs stay honest.

## Dependencies

If your code samples need crates, configure them in `dodeca.styx`:

```styx
code_execution {
    dependencies (
        {name serde, version "1.0"}
    )
}
```

## Disabling

Set the `DODECA_NO_CODE_EXEC=1` environment variable to skip code execution (useful for quick iterations when you're not editing code blocks).

## How it works

During `ddc build` and `ddc serve`, code blocks marked with `test` are extracted, compiled with `rustc`, and executed. The build fails noisily if any code block doesn't compile or panics — no silent failures.
