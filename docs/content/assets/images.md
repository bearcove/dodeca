+++
title = "Images"
weight = 10
+++

dodeca automatically processes images in your content into responsive, modern formats.

## How it works

When you use a standard markdown image:

```markdown
![A mountain](/images/mountain.jpg)
```

dodeca generates:

- **Multiple widths**: 320, 640, 960, 1280, and 1920 pixels (only sizes smaller than the original, plus the original size)
- **Two modern formats**: JPEG XL (`.jxl`) and WebP (`.webp`)
- **Thumbhash placeholder**: A tiny inline data URL for instant loading

The markdown image becomes a `<picture>` element with `<source>` sets for each format and a `srcset` with all available widths.

## Supported input formats

PNG, JPEG, GIF, WebP, and JPEG XL.

SVGs are not rasterized — they're processed separately (see [SVG Optimization](/assets/svg-optimization/)).

## Output

Every image variant gets a content-hashed filename and is stored in the build cache (`.cache/blobs/`). On subsequent builds, only changed images are reprocessed.

## No configuration needed

Image processing is automatic. There's nothing to configure — drop images into your content or `static/` directory and reference them normally.
