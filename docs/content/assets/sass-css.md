+++
title = "SASS & CSS"
weight = 30
+++

## SASS compilation

Place SASS/SCSS files in the `sass/` directory. `sass/main.scss` is the entry point.

Partials (files starting with `_`, like `_variables.scss`) can be imported but aren't compiled independently:

```scss
// sass/main.scss
@use "variables";
@use "layout";
@use "components";
```

## CSS processing

Compiled CSS is processed through lightningcss for:

- Vendor prefixing
- Minification
- `url()` rewriting to cache-busted paths

## URL rewriting

References to other assets inside your CSS are automatically rewritten:

```css
/* You write: */
background: url("/images/bg.png");

/* dodeca outputs: */
background: url("/images/bg.a7x3k2.png");
```

This happens transparently for all `url()` values in your stylesheets.
