+++
title = "Incremental Computation"
description = "How dodeca tracks dependencies and only rebuilds what changed"
weight = 1
+++

dodeca uses [Picante](https://picante.bearcove.eu/), an async incremental query runtime. Everything is a query; queries are memoized async functions that track which inputs they read.

When you change a file, picante traces exactly which outputs depend on it and only recomputes those. Edit `features.md` and only that page re-renders—other pages, CSS, images, and fonts stay cached.

```
SourceFile → parse_file → build_tree → render_page → all_rendered_html → build_site
                                ↑
TemplateFile → load_template ───┘
```

The cache persists to disk via [facet-postcard](https://github.com/facet-rs/facet). Even a cold start benefits from previous work. Large outputs (images, fonts) are stored separately in content-addressed storage (CAS) to keep the picante database small.

See [Architecture](@/internals/architecture.md) for how picante, CAS, and plugins work together.
