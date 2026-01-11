+++
title = "Static Assets"
description = "How to manage images, fonts, and other static files"
weight = 35
+++

Static assets are files served directly without Markdown processing: images, fonts, CSS, JavaScript, PDFs, etc.

## Directory structure

Place static files in a `static/` directory alongside your `content/` directory:

```
my-site/
├── .config/
│   └── dodeca.yaml
├── content/
│   └── ...
├── static/
│   ├── favicon.svg
│   ├── robots.txt
│   ├── images/
│   │   └── logo.png
│   └── fonts/
│       └── Inter.woff2
└── templates/
    └── ...
```

Files in `static/` are available at the root of your site:
- `static/favicon.svg` → `/favicon.svg`
- `static/images/logo.png` → `/images/logo.png`

## Cache busting

By default, dodeca adds a content hash to static file URLs for cache busting:

```
/images/logo.png → /images/logo.0a3dec24oc21.png
```

This ensures browsers fetch fresh versions when files change, while allowing aggressive caching for unchanged files.

## Referencing assets

### In templates

Use the `get_url` function:

```jinja
<link rel="icon" href="{{ get_url(path="favicon.svg") }}">
<img src="{{ get_url(path="images/logo.png") }}" alt="Logo">
```

URLs are automatically rewritten to their cache-busted versions.

### In Markdown

Use standard Markdown syntax:

```markdown
![Logo](/images/logo.png)
```

Or HTML for more control:

```html
<img src="/images/logo.png" alt="Logo" width="200">
```

## Stable assets

Some assets need stable URLs that don't change:

- `favicon.svg` (browsers cache by exact URL)
- `robots.txt` (search engines expect fixed location)
- `og-image.png` (social media caches preview images)

Configure these in `.config/dodeca.yaml`:

```yaml
stable_assets:
  - favicon.svg
  - robots.txt
  - og-image.png
```

Stable assets are served at both their original path and cache-busted path.

## Asset processing

Dodeca automatically optimizes certain asset types:

### Images

Images are converted to modern formats with responsive sizes:

| Original | Generated |
|----------|-----------|
| `photo.png` | `photo.jxl`, `photo.webp` at multiple widths |
| `photo.jpg` | Same as above |

In HTML, `<img>` tags are transformed to `<picture>` elements with `srcset` for responsive loading.

### Fonts

Font files (`.woff2`, `.woff`, `.ttf`, `.otf`) are analyzed and subsetted to include only the characters actually used in your site, reducing file size.

### SVGs

SVG files are optimized using SVGO to remove unnecessary metadata and whitespace.

### CSS

URLs inside CSS files are rewritten to point to cache-busted asset paths.

## Example configuration

```yaml
content: content
output: public

stable_assets:
  - favicon.svg
  - robots.txt
  - og-image.png
  - apple-touch-icon.png
```

## Best practices

1. **Use SVG for icons and logos** - They scale perfectly and are usually smaller than PNGs

2. **Use WOFF2 for fonts** - Best compression for web fonts

3. **Keep originals in `static/`** - Dodeca generates optimized versions automatically

4. **Mark external-facing assets as stable** - Favicon, robots.txt, Open Graph images

5. **Use descriptive paths** - `/images/hero-banner.png` not `/img1.png`
