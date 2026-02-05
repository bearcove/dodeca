+++
title = "Cache Busting"
weight = 21
+++

Every asset except HTML pages gets a content hash in its filename: `style.css` becomes `style.kj7m2x.css`. All references are rewritten to match, so you can serve assets with `Cache-Control: immutable` and never worry about stale caches.

```html
<link href="/css/style.kj7m2x.css">
<img src="/images/logo.m4n8p2.png">
```

CSS `url()` values are rewritten too:

```css
@font-face { src: url('/fonts/inter.x9k3j1.woff2'); }
```

Some files need stable paths (favicons, robots.txt). Configure them in `dodeca.styx`:

```yaml
stable_assets:
  - favicon.svg
  - robots.txt
```

When a file changes, only that file is re-hashed, and only pages referencing it are regenerated.
