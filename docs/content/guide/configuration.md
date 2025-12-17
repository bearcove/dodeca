+++
title = "Configuration"
description = "Configuring dodeca"
weight = 30
+++

Configuration lives in `.config/dodeca.kdl` in your project root.

## Basic settings

```kdl
content "content"
output "public"
```

## Link checking

`ddc build` checks links and fails the build if it finds broken ones.

If you have a lot of external links (or you’re hitting rate limits / anti-bot checks), you can tune external checking:

```kdl
link_check {
    # Skip domains entirely (useful for sites that block bots)
    skip_domain "example.com"

    # Minimum delay between requests to the same domain
    rate_limit_ms 1000
}
```

## Code execution

If the code execution helper is available (`ddc-cell-code-execution`), Rust code blocks can be executed as part of the build.

At the moment, `.config/dodeca.kdl` contains a `code_execution { ... }` section, but it is not fully wired through to the build yet (defaults are used). If you need to disable code execution, use the environment variable `DODECA_NO_CODE_EXEC=1`.

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
content "content"
output "public"

link_check {
    skip_domain "example.com"
    rate_limit_ms 1000
}

stable_assets {
    path "favicon.svg"
}
```
