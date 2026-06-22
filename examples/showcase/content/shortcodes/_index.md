---
title: Shortcodes
---

The three shortcode grammars marq supports, rendered through gingembre templates
in `templates/shortcodes/`:

- **fenced** `+++ :name: <yaml> +++` — block, no body (e.g. `youtube`)
- **body** `> *:name(args)* + body` — blockquote with a rendered-markdown body (e.g. `tip`, `bearsays`)
- **inline** `*:name(args)*` — mid-paragraph, no body
