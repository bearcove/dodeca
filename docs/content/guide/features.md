+++
title = "Features"
description = "What dodeca offers"
weight = 40
+++

## Build

- **Incremental builds** via [Salsa](https://salsa-rs.github.io/salsa/) - only rebuild what changed
- **Sass/SCSS compilation** - modern CSS workflow built-in
- **Search indexing** via Pagefind - fast client-side search
- **Link checking** - catch broken internal and external links

## Runtime

- **DOM patching** - no full reload, just surgical DOM updates via WASM
- **Live-reload dev server** - instant feedback while editing

## Assets

- **Font subsetting** - only include glyphs actually used on your site (saves TONS of bandwidth!)
- **Responsive images** - automatic JXL/WebP variants at multiple sizes

## Templating

- **Jinja-like template engine** - familiar syntax, zero serde

## Code Sample Execution

- **Automatic validation** - all code samples in your markdown are executed during build
- **Build failure protection** - builds fail if any code samples don't work (in production mode)
- **Multi-language support** - currently Rust, with Python/JavaScript planned
- **Dependency management** - configure custom dependencies for your examples
- **Performance optimized** - Blake3 content hashing for fast incremental builds
- **Detailed error reporting** - see exactly which file, line, and what went wrong

### Automatic Rust Validation

Any fenced Rust code block is automatically compiled and executed:

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

```rust,noexec
let message = "Hello, world!";
println!("{}", message);
// This becomes fn main() { ... } automatically
```

### Error Reporting

When code fails, you get detailed feedback:

```
✗ Code execution failed in content/guide/example.md:25 (rust): Process exited with code: Some(1)
  stderr: error[E0425]: cannot find value `undefined_var` in this scope
```

See the [Code Execution Guide](./code-execution.md) for complete documentation.
