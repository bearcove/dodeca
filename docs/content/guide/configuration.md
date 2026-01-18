+++
title = "Configuration"
description = "Configuring dodeca"
weight = 30
+++

Configuration lives in `.config/dodeca.styx` in your project root.

> YAML format (`.config/dodeca.yaml`) is also supported for backwards compatibility.

## Basic settings

```styx
content content
output public
```

## Schema support

Dodeca supports Styx schema discovery via CLI. When you have `ddc` installed, editors can run `ddc @dump-styx-schema` to discover the config schema automatically for autocomplete and validation.

## Link checking

`ddc build` checks all internal and external links, failing the build if any are broken.

### Configuration options

```styx
link_check {
    rate_limit_ms 1500
    skip_domains (linkedin.com twitter.com x.com)
}
```

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `rate_limit_ms` | integer | `1000` | Minimum delay between requests to the same domain (milliseconds) |
| `skip_domains` | list of strings | none | Domains to skip checking entirely |

### When to skip domains

Some sites aggressively block automated requests. Consider skipping:

- **Social media**: `linkedin.com`, `twitter.com`, `x.com`, `facebook.com`
- **Sites with CAPTCHAs**: Some documentation sites require JavaScript
- **Rate-limited APIs**: Sites that return 429 errors frequently

### Example with multiple skip domains

```styx
link_check {
    rate_limit_ms 2000
    skip_domains (
        linkedin.com
        twitter.com
        x.com
        facebook.com
        instagram.com
    )
}
```

Internal links (links to other pages in your site) are always checked and cannot be skipped.

## Code execution

If the code execution helper is available (`ddc-cell-code-execution`), Rust code blocks can be executed as part of the build.

At the moment, `.config/dodeca.styx` contains a `code_execution` section, but it is not fully wired through to the build yet (defaults are used). If you need to disable code execution, use the environment variable `DODECA_NO_CODE_EXEC=1`.

## Stable assets

Some assets (like favicons) should keep stable URLs for caching:

```styx
stable_assets (favicon.svg robots.txt)
```

## Full example

```styx
content content
output public

link_check {
    rate_limit_ms 1000
    skip_domains (example.com)
}

stable_assets (favicon.svg)
```
