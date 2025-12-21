+++
title = "Features"
description = "What dodeca offers"
weight = 40
+++

This page describes what’s currently implemented in dodeca (as shipped in this repository). Some features depend on helper binaries (`ddc-cell-*`); if those binaries are missing, the corresponding feature may be unavailable.

## Build

- **Incremental builds** via picante - only rebuild what changed
- **Sass/SCSS compilation** - modern CSS workflow built-in
- **Search indexing** via Pagefind (generates static search files)
- **Link checking** - fail builds on broken links

## Runtime

- **Live reload dev server** - rebuilds on change and notifies the browser
- **DOM patching** - can update pages without a full reload (when live reload is enabled)

## Assets

- **Font subsetting** - include only glyphs used by your site
- **Responsive images** - generate multiple sizes and modern formats (e.g. WebP/JXL)

## Templating

- **Jinja-like template engine** - blocks, inheritance, loops, filters, and diagnostics

## Code Sample Execution

- **Rust code blocks** can be executed during the build (if `ddc-cell-code-execution` is available)
- `ddc build` fails on execution errors; `ddc serve` reports them as warnings
- Add `,noexec` to a fenced Rust code block to skip execution (for pseudo-code or intentionally failing examples)

### Automatic Rust Validation

Any fenced Rust code block is treated as executable:

```rust
use std::collections::HashMap;

fn main() {
    let mut scores = HashMap::new();
    scores.insert("Alice", 10);
    scores.insert("Bob", 8);

    for (name, score) in &scores {
        println!("{name}: {score}");
    }
}
```

### Auto-wrapped Code

Code without a main function is automatically wrapped:

```rust
let message = "Hello, world!";
println!("{}", message);
```

### Error Reporting

When code fails, you get detailed feedback:

```
✗ Code execution failed in content/guide/example.md:25 (rust): Process exited with code: Some(1)
  stderr: error[E0425]: cannot find value `undefined_var` in this scope
```

See the [Code Execution Guide](./code-execution.md) for complete documentation.
