+++
title = "Frontmatter Reference"
weight = 20
+++

All frontmatter fields available in content files.

## Page fields (non-`_index.md` files)

| Field | Type | Default | Template access |
|-------|------|---------|-----------------|
| `title` | string | `""` | `page.title` |
| `weight` | integer | `0` | `page.weight` |
| `path` | string | *from filename* | `page.path` |
| `extra` | table | `{}` | `page.extra` |
| `extra.description` | string | — | `page.description` |

`page.path` is the URL route (e.g. `/blog/my-post/`), not the source file path.

`page.last_updated` is derived from the file's modification time.

## Section fields (`_index.md` files)

| Field | Type | Default | Template access |
|-------|------|---------|-----------------|
| `title` | string | `""` | `section.title` |
| `weight` | integer | `0` | `section.weight` |
| `extra` | table | `{}` | `section.extra` |

## Computed fields

These are not set in frontmatter but are available in templates:

| Field | Available on | Description |
|-------|-------------|-------------|
| `*.content` | page, section | Rendered markdown HTML |
| `*.permalink` | page, section | Full URL path |
| `*.toc` | page, section | Table of contents HTML |
| `*.ancestors` | page, section | Array of parent section paths |
| `*.last_updated` | page, section | File modification time |
| `section.pages` | section | Child pages (sorted by weight) |
| `section.subsections` | section | Child sections (sorted by weight) |
