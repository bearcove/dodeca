+++
title = "Configuration"
description = "Configuring dodeca"
weight = 30
+++

Configuration lives in `.config/dodeca.kdl` in your project root.

## Basic settings

```kdl
content "docs/content"
output "docs/public"
```

## Link checking

Enable broken link detection:

```kdl
link_check {
}
```

## Code execution

Control automatic code sample validation:

```kdl
code_execution {
    enabled true
    fail_on_error false
    dependency "serde" version="1.0" features=["derive"]
}
```

Common options:
- `enabled false` - Turn off code execution entirely
- `fail_on_error true` - Fail builds on broken code even in dev mode  
- `dependency` - Add crates your code examples need

## Stable assets

Some assets (like favicons) should keep stable URLs for caching:

```kdl
stable_assets {
    path "favicon.svg"
    path "robots.txt"
}
```

## Full example

```kdl
content "docs/content"
output "docs/public"

link_check {
}

code_execution {
    dependency "serde" version="1.0" features=["derive"]
}

stable_assets {
    path "favicon.svg"
}
```
