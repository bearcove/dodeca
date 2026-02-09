+++
title = "Directory Structure"
weight = 10
+++

A dodeca project looks like this:

```
my-site/
├── .config/
│   └── dodeca.styx        # Site configuration
├── content/               # Markdown content
│   ├── _index.md          # Root section
│   ├── blog/
│   │   ├── _index.md      # Blog section
│   │   ├── first-post.md  # Blog page
│   │   └── second-post.md
│   └── about.md           # Standalone page
├── data/                  # Data files (JSON, TOML, YAML)
├── sass/                  # SASS stylesheets
│   └── main.scss          # Entry point
├── static/                # Copied as-is (before cache busting)
│   ├── favicon.svg
│   └── fonts/
├── templates/             # HTML templates
│   ├── base.html
│   ├── index.html
│   ├── section.html
│   └── page.html
└── public/                # Build output (configurable)
```

## `.config/dodeca.styx`

Site-wide configuration. See [Configuration](/reference/configuration/).

## `content/`

Markdown files with TOML frontmatter. The directory structure maps to URL routes:

| File | URL |
|------|-----|
| `content/_index.md` | `/` |
| `content/about.md` | `/about/` |
| `content/blog/_index.md` | `/blog/` |
| `content/blog/first-post.md` | `/blog/first-post/` |

## `data/`

JSON, TOML, or YAML files accessible in templates via the `data` context variable. The filename (without extension) becomes the key.

## `sass/`

SASS/SCSS files. `sass/main.scss` is the entry point. Partials (files starting with `_`) are available for `@import` / `@use` but not compiled independently.

## `static/`

Files copied to the output directory. These go through cache busting — `url()` references in CSS and `src`/`href` in HTML are rewritten to content-hashed paths.

Use `stable_assets` in `dodeca.styx` for files that need fixed paths (like `favicon.ico` or `robots.txt`).

## `templates/`

Gingembre templates. See [Template Basics](/templates/basics/).
