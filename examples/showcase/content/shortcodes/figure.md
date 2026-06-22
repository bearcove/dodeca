---
title: Figure (fenced, with a real image)
---

The `figure` shortcode is the highest-volume one on the real site (≈191 uses). It uses
`get_media(src).markup(...)` for a responsive image plus a markdown caption — so this page
exercises the whole asset pipeline: `<img>` → responsive `<picture>` (JXL + WebP +
thumbhash), cache-busted URLs, and the picante asset dependency.

+++
:figure:
  src: /img/sample.png
  title: The **gingembre** logo, rendered through the `figure` shortcode.
  alt: gingembre logo
  attr: dodeca
  attrlink: https://github.com/bearcove/dodeca
+++

If the image above is a responsive `<picture>` with a caption, the figure shortcode and the
image pipeline are both working end-to-end.
