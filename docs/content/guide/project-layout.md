+++
title = "Project Layout"
description = "How a dodeca site is structured on disk"
weight = 15
+++

A minimal dodeca site is a directory with:

- `.config/dodeca.styx` (required)
- `content/` (required)
- `templates/` (required)

Common optional directories:

- `static/` for files copied through to output (images, robots.txt, etc.)
- `sass/` for `sass/main.scss` (if you want SCSS compilation)
- `data/` for structured data files loaded into templates

## Example layout

```
my-site/
├── .config/
│   └── dodeca.styx
├── content/
│   ├── _index.md
│   └── guide/
│       ├── _index.md
│       └── intro.md
├── templates/
│   ├── base.html
│   ├── index.html
│   ├── section.html
│   └── page.html
├── static/
│   └── images/
│       └── logo.png
└── public/               # build output (configured via dodeca.styx)
```

## Content model

- A folder under `content/` is a *section*.
- A section’s landing page/content comes from `_index.md` inside that folder.
- Other Markdown files in a section are *pages*.

Markdown files can start with TOML frontmatter delimited by `+++`:

```toml
+++
title = "Intro"
description = "Optional"
weight = 10
+++
```

## Templates

dodeca looks for these templates under `templates/`:

- `index.html` renders the root section (`/`)
- `section.html` renders non-root sections
- `page.html` renders individual pages

Templates can use `{% extends "base.html" %}` and blocks. See [Template Engine](/internals/templates/) for the full template/context reference.
