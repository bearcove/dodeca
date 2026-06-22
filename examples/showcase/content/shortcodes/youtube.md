---
title: YouTube (fenced)
---

The fenced grammar — a `+++` block whose first line is a `:name:` key. The whole
block is handed to the resolver as YAML, so nested keys become template variables
(`id`, `alt`).

+++
:youtube:
  id: dQw4w9WgXcQ
  alt: A famously persistent music video
+++

This one renders fully today: it uses external YouTube thumbnails and needs no
`get_media`.
