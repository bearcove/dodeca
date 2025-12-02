+++
title = "Minification"
weight = 33
+++

Production builds (`ddc build`) minify everything. Dev server keeps output readable unless you pass `--release`.

**HTML** via [minify-html](https://github.com/nickmass/minify-html): collapses whitespace, omits optional tags, strips comments, removes unnecessary quotes.

```html
<div class=container><p>Hello, world!
```

**CSS** via [lightningcss](https://lightningcss.dev/): merges rules, shortens colors, eliminates dead code, handles vendor prefixes.

**JavaScript** via [OXC](https://oxc.rs/): renames variables, folds constants, eliminates dead code, tree shakes.

**SVG** via [svgcleaner](https://github.com/nickmass/svag): removes metadata, collapses groups, optimizes paths.
