+++
title = "Fonts"
weight = 20
+++

dodeca automatically subsets fonts to only the characters used on your site, powered by [fontcull](https://github.com/bearcove/fontcull).

## How it works

1. All rendered HTML is scanned for used characters
2. Each font file is subsetted to only those glyphs
3. The subsetted fonts get content-hashed filenames
4. All CSS `@font-face` references are rewritten

This typically removes 90%+ of the font file size. Variable fonts are preserved.

## Usage

Place font files in your `static/fonts/` directory and reference them in CSS:

```css
@font-face {
    font-family: "My Font";
    src: url("/fonts/MyFont-Regular.woff2") format("woff2");
    font-weight: 400;
    font-style: normal;
}
```

dodeca handles the rest — subsetting, cache busting, and URL rewriting all happen automatically.
