# `docs/`

This directory contains the documentation website for dodeca.

## What’s what

- `docs/content/`: Markdown content (the source of the docs site)
- `docs/templates/`: HTML templates used to render the docs site
- `docs/static/`: Static assets (images, fonts, JS/CSS sources, etc.)
- `docs/sass/`: Sass sources for the docs site
- `docs/public/`: Generated output (what `ddc build` writes)

## Preview locally

From the repository root:

- `ddc build` writes output to `docs/public/` (per `.config/dodeca.kdl`).
- `ddc serve` runs the dev server and live reload.

## Notes

- `docs/public/` is a build artifact. If you want to change the docs site, edit `docs/content/`, `docs/templates/`, `docs/static/`, or `docs/sass/` instead.
