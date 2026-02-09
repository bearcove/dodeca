+++
title = "Static Files & Cache Busting"
weight = 50
+++

## Static files

Files in the `static/` directory are copied to the output. Their content is hashed and the filename is rewritten:

```
static/css/style.css  →  css/style.kj7m2x.css
static/js/app.js      →  js/app.p3n8q1.js
```

All references to these files in HTML (`src`, `href`) and CSS (`url()`) are automatically rewritten to use the hashed names.

## How cache busting works

Every asset gets a content-hash inserted into its filename. When the file content changes, the hash changes, and browsers fetch the new version. When it hasn't changed, browsers serve from cache.

This means you can set aggressive cache headers (`Cache-Control: max-age=31536000`) on all assets.

## Stable assets

Some files need fixed paths — like `favicon.ico`, `robots.txt`, or `CNAME`. Configure these in `dodeca.styx`:

```styx
stable_assets (
    favicon.ico
    favicon.svg
    robots.txt
    CNAME
)
```

These files are copied without renaming.
