+++
title = "Frontmatter"
weight = 30
+++

Every content file starts with TOML frontmatter between `+++` delimiters:

```markdown
+++
title = "My Page"
weight = 10
+++

Page content goes here.
```

## Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `title` | string | `""` | Page or section title |
| `weight` | integer | `0` | Sort order (ascending) |
| `path` | string | *from filename* | URL route override |
| `extra` | table | `{}` | Arbitrary key-value data |

## Extra fields

The `extra` table is where you put custom data:

```markdown
+++
title = "My Page"

[extra]
description = "A description for meta tags"
author = "Someone"
show_toc = true
+++
```

Access these in templates as `page.extra.description`, `page.extra.author`, etc.

`page.description` is a special case — if `extra.description` is set, it's also available directly as `page.description` for Zola compatibility.

## What's available in templates

See [Context Variables](/templates/context-variables/) for the full list of what's accessible as `page.*` and `section.*` in templates.

See [Frontmatter Reference](/reference/frontmatter-reference/) for a complete lookup table.
