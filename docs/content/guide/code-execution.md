+++
title = "Code Execution"
description = "Automatic code sample validation"
weight = 45
+++

Dodeca automatically runs code samples in your markdown files to make sure they actually work. No more embarrassing docs with broken examples!

## How it works

When you write fenced code blocks like this:

````markdown
```rust
let name = "Alice";
println!("Hello, {}!", name);
```
````

Dodeca will:
1. Extract the code during your build
2. Run it to make sure it works
3. Fail your build if the code is broken

This happens automatically - no setup required.

## What gets executed

By default, Rust code blocks are executed. Other languages are planned.

### Simple examples

```rust
let x = 5 + 3;
println!("Result: {}", x);
```

This gets wrapped in a `main()` function automatically.

### Complete programs

```rust
fn greet(name: &str) {
    println!("Hello, {}!", name);
}

fn main() {
    greet("World");
}
```

This runs as-is.

## Skipping execution

Sometimes you don't want code to run (pseudo-code, broken examples for teaching, etc.):

````markdown
```rust,noexec
// This won't be executed
let broken_code = does_not_compile();
```
````

Or disable for a whole file in frontmatter:

```markdown
+++
title = "My Page"
code_execution = false
+++
```

## When builds fail

If your code doesn't work, the build stops:

```
✗ Code execution failed in content/tutorial.md:42 (rust): Process exited with code: Some(1)
  stderr: error[E0425]: cannot find value `typo_variable`
```

Fix the code and rebuild. In development mode (`ddc serve`), you get warnings instead of hard failures.

## Configuration

Add to `.config/dodeca.kdl` to customize behavior:

```kdl
code_execution {
    # Turn off completely
    enabled false
    
    # Fail builds even in dev mode  
    fail_on_error true
    
    # Add dependencies for your examples
    dependency "serde" version="1.0" features=["derive"]
}
```

## Performance

Code execution is cached - unchanged samples don't re-run on subsequent builds. Your incremental builds stay fast.

## Best practices

**Keep examples focused:**
- Show one concept per code block
- Avoid complex setup code
- Use realistic but minimal examples

**Test your docs:**
- Run `ddc build` before publishing
- Let CI catch broken examples for you
- Update examples when APIs change

**For complex examples:**
- Break into smaller pieces
- Link to full example projects in your repo
- Use configuration to add needed dependencies

That's it! Your documentation examples now stay working automatically.