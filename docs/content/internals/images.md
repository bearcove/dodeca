+++
title = "Image Processing"
weight = 23
+++

```mermaid
flowchart LR
    SRC["Source image<br/>(PNG/JPEG)"] --> META[image_metadata]
    META --> HASH[image_input_hash]
    HASH --> PROC[process_image]
    PROC --> JXL["400w / 800w / 1200w<br/>.jxl"]
    PROC --> WEBP["400w / 800w / 1200w<br/>.webp"]
    PROC --> JPEG["400w / 800w / 1200w<br/>.jpg (fallback)"]
    JXL --> PIC["&lt;picture&gt; element"]
    WEBP --> PIC
    JPEG --> PIC
```

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

Processed images are cached in `.cache/blobs/`. Unchanged images skip reprocessing entirely.
