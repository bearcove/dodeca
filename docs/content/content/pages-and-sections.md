+++
title = "Pages & Sections"
weight = 20
+++

dodeca has two kinds of content: **pages** and **sections**.

## Sections

A section is created by placing an `_index.md` file in a directory:

```
content/
├── _index.md           # Root section (/)
├── blog/
│   ├── _index.md       # Blog section (/blog/)
│   └── ...
└── docs/
    ├── _index.md       # Docs section (/docs/)
    └── guide/
        └── _index.md   # Nested section (/docs/guide/)
```

Sections use the `section.html` template (or `index.html` for the root section). In templates, a section has access to its `pages` and `subsections`.

## Pages

Any `.md` file that isn't `_index.md` is a page:

```
content/
├── about.md            # Page at /about/
└── blog/
    ├── _index.md       # Section
    ├── hello.md        # Page at /blog/hello/
    └── world.md        # Page at /blog/world/
```

Pages use the `page.html` template.

## Ordering with `weight`

Pages and sections are sorted by the `weight` frontmatter field (ascending). Lower weight = appears first.

```markdown
+++
title = "First Item"
weight = 10
+++
```

```markdown
+++
title = "Second Item"
weight = 20
+++
```

## Routing

The URL is derived from the file's path relative to `content/`:

- `content/blog/my-post.md` → `/blog/my-post/`
- `content/blog/_index.md` → `/blog/`
- `content/_index.md` → `/`

The frontmatter `path` field can override this if needed.
