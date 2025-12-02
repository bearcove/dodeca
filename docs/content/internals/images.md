+++
title = "Image Processing"
weight = 23
+++

A single markdown image becomes a responsive `<picture>` with JPEG XL, WebP, and JPEG fallbacks at multiple sizes:

```html
<picture>
  <source srcset="/images/mountain.400w.jxl 400w,
                  /images/mountain.800w.jxl 800w,
                  /images/mountain.1200w.jxl 1200w"
          type="image/jxl">
  <source srcset="..." type="image/webp">
  <img src="/images/mountain.800w.jpg"
       srcset="..."
       alt="Mountain landscape"
       loading="lazy">
</picture>
```

Browsers pick the best format they support. All images get `loading="lazy"`.

Processed images are cached in `.cache/images.canopy`. Unchanged images skip reprocessing entirely.

dodeca also generates Open Graph images for social sharing using [resvg](https://github.com/RazrFalcon/resvg), rendered from SVG templates.
