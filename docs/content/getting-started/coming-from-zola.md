+++
title = "Coming from Zola"
weight = 30
+++

dodeca's content model is intentionally Zola-compatible. If you've used Zola, most concepts transfer directly.

## What's the same

- **Content structure** — `content/` directory with `_index.md` for sections, other `.md` files for pages
- **TOML frontmatter** — `+++` delimiters, same fields like `title`, `weight`, `extra`
- **Template inheritance** — `{% extends "base.html" %}`, `{% block %}`, `{% include %}`
- **Section/page distinction** — `_index.md` creates a section, everything else is a page
- **Familiar syntax** — Jinja-like templates with `{{ }}` and `{% %}`

## What's different

### Configuration

Zola uses `config.toml`. dodeca uses `.config/dodeca.styx` with [Styx](https://github.com/bearcove/styx) syntax:

```styx
content content
output public

syntax_highlight {
    light_theme github-light
    dark_theme tokyo-night
}
```

Styx uses `key value` pairs — no colons, no equals signs. See [Configuration](/reference/configuration/).

### Templates

dodeca uses gingembre instead of Tera. The syntax is nearly identical (Jinja-like), but there are differences in available filters, functions, and tests. See the [Template Reference](/reference/template-reference/).

### `config.title` and `config.description`

In dodeca, these come from the root `_index.md` frontmatter, not from the config file:

```markdown
+++
title = "My Site"
description = "A site about things"
+++
```

### Assets are processed automatically

In Zola, you opt into image resizing with `resize_image()`. In dodeca:

- **Images** are automatically converted to JXL + WebP at multiple widths with `<picture>` elements
- **Fonts** are automatically subsetted to only the characters used on your site
- **All assets** get content-hash filenames for cache busting
- **SASS** is compiled and CSS is processed automatically

### Dev = Production

Zola's dev server skips some production processing. dodeca doesn't — what you see in `ddc serve` is exactly what `ddc build` produces. This means no surprises at deploy time.
