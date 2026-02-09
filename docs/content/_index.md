+++
title = "dodeca"
description = "A fully incremental static site generator"
+++

**Dev mode = production mode.** dodeca serves exactly what you'll deploy — cache-busted URLs, responsive images, subsetted fonts — even during development. Change a file and only the affected parts rebuild. The browser updates via DOM patching, no full-page reloads.

## Highlights

- **Incremental** — powered by [picante](https://github.com/bearcove/picante), only affected queries re-run on changes
- **Live reload** — DOM patches via [hotmeal](https://github.com/bearcove/hotmeal), not full-page reloads
- **Production-ready in dev** — cache-busted URLs, responsive images, subsetted fonts, all the time
- **Vite integration** — first-class support for modern frontend tooling
- **Zola-compatible content** — TOML frontmatter, `_index.md` sections, familiar directory structure
