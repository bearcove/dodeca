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

stable_assets {
    path "favicon.svg"
}
```
