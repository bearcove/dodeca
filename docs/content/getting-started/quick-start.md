+++
title = "Quick Start"
weight = 20
+++

## Create a new site

```bash
ddc init my-site
cd my-site
```

`ddc init` creates a project with a starter template, content directory, and configuration file.

## Start the dev server

```bash
ddc serve
```

This starts a development server with a TUI dashboard. Your site is available at the URL shown in the terminal. Press `q` to quit.

If you prefer plain terminal output:

```bash
ddc serve --no-tui
```

## Edit content

Open `content/_index.md` in your editor. Change something. Save. The browser updates instantly — dodeca patches the DOM directly, no full-page reload needed.

## Build for production

```bash
ddc build
```

Output goes to the directory configured in `.config/dodeca.styx` (typically `public/`). This is exactly what `ddc serve` was already serving — same cache-busted URLs, same responsive images, same subsetted fonts.

## Project structure

After `ddc init`, you'll have:

```
my-site/
├── .config/
│   └── dodeca.styx      # Site configuration
├── content/
│   └── _index.md        # Root section (homepage)
├── templates/
│   ├── base.html        # Base layout
│   ├── index.html       # Homepage template
│   ├── section.html     # Section template
│   └── page.html        # Page template
└── static/              # Files copied as-is
```

See [Directory Structure](/content/directory-structure/) for the full breakdown.
