+++
title = "Salsa"
description = "How dodeca tracks dependencies and only rebuilds what changed"
weight = 1
+++

dodeca is built on [Salsa](https://salsa-rs.github.io/salsa/). Everything is a query; queries are memoized functions that track which inputs they read.

When you change a file, Salsa traces exactly which outputs depend on it and only recomputes those. Edit `features.md` and only that page re-renders—other pages, CSS, images, and fonts stay cached.

```
SourceFile → parse_file → build_tree → render_page → all_rendered_html → build_site
                                ↑
TemplateFile → load_template ───┘
```

The cache persists to disk. Even a cold start benefits from previous work. Large outputs (images, fonts) are stored separately in content-addressed storage to keep the Salsa database small.
