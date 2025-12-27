+++
title = "Frontmatter Reference"
description = "Complete reference for page and section frontmatter fields"
weight = 30
+++

Frontmatter is metadata at the top of Markdown files that controls how pages and sections are rendered.

## Format

Frontmatter uses TOML format delimited by `+++`:

```toml
+++
title = "My Page Title"
description = "A brief description"
weight = 10

[extra]
author = "amos"
+++

# Markdown content starts here
```

## Supported Fields

### title

| Type | Default |
|------|---------|
| `string` | `""` (empty) |

The title of the page or section. Available as `page.title` or `section.title` in templates.

```toml
+++
title = "Getting Started"
+++
```

### description

| Type | Default |
|------|---------|
| `string` (optional) | none |

An optional description. Available as `section.description` in templates (sections only).

```toml
+++
title = "API Reference"
description = "Complete API documentation for all endpoints"
+++
```

### weight

| Type | Default |
|------|---------|
| `integer` | `0` |

Sort order for pages and sections. Lower weights appear first. Available as `page.weight` or `section.weight`.

```toml
+++
title = "Introduction"
weight = 1
+++
```

```toml
+++
title = "Advanced Topics"
weight = 99
+++
```

### template

| Type | Default |
|------|---------|
| `string` (optional) | none |

Custom template to use for rendering. If not specified, dodeca uses:
- `index.html` for the root section
- `section.html` for other sections
- `page.html` for pages

```toml
+++
title = "Landing Page"
template = "landing.html"
+++
```

### [extra]

| Type | Default |
|------|---------|
| TOML table | `{}` (empty) |

Custom fields accessible as `page.extra.*` or `section.extra.*` in templates. Supports any valid TOML types including nested objects.

```toml
+++
title = "Blog Post"

[extra]
author = "amos"
reading_time = 5
tags = ["rust", "tutorial"]
metadata = { version = "1.0", updated = "2024-01-15" }
+++
```

In templates:

```jinja
<p>By {{ page.extra.author }}</p>
<p>{{ page.extra.reading_time }} min read</p>
{% for tag in page.extra.tags %}
  <span class="tag">{{ tag }}</span>
{% endfor %}
```

## Complete Example

```toml
+++
title = "Comprehensive Guide"
description = "Everything you need to know"
weight = 10
template = "guide.html"

[extra]
author = "amos"
difficulty = "intermediate"
prerequisites = ["basics", "setup"]
sidebar = true
+++
```

## Automatic Fields

These fields are automatically derived and available in templates:

| Field | Description |
|-------|-------------|
| `page.path` | URL path (e.g., `/guide/intro/`) |
| `page.content` | Rendered HTML content |
| `page.word_count` | Number of words |
| `page.reading_time` | Estimated reading time in minutes |
| `page.toc` | Table of contents (list of headings) |
| `section.pages` | Child pages sorted by weight |
| `section.subsections` | Child sections sorted by weight |

## Notes

- Only TOML frontmatter with `+++` delimiters is supported (not YAML with `---`)
- Date fields are not built-in; use `[extra]` for custom date handling
- File modification time is tracked automatically via `last_updated`
