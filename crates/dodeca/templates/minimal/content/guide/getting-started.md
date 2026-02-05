+++
title = "Getting Started"
weight = 1
+++

## Your First Page

You're reading your first page! Edit this file at `content/guide/getting-started.md`.

## Project Structure

```
{{site_name}}/
├── .config/
│   └── dodeca.styx     # Configuration
├── content/
│   ├── _index.md       # Home page
│   └── guide/
│       ├── _index.md   # Guide section
│       └── getting-started.md
├── sass/
│   └── main.scss       # Styles
├── static/             # Static assets
└── templates/
    ├── base.html       # Base template
    ├── index.html      # Home page template
    ├── section.html    # Section template
    └── page.html       # Page template
```

## Adding Content

Create new Markdown files in `content/` with frontmatter:

```markdown
+++
title = "My New Page"
weight = 10
+++

Your content here...
```

## Running the Dev Server

```bash
ddc serve --open
```

Changes are automatically reloaded in your browser.

## Building for Production

```bash
ddc build
```

Output is written to `public/`.
