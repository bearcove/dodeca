+++
title = "Minification"
weight = 33
+++

Production builds (`ddc build`) minify everything. Dev server keeps output readable unless you pass `--release`.

**CSS** via [lightningcss](https://lightningcss.dev/): merges rules, shortens colors, eliminates dead code, handles vendor prefixes.

**JavaScript** via [OXC](https://oxc.rs/): renames variables, folds constants, eliminates dead code, tree shakes.

**SVG** via [svag](https://github.com/bearcove/svag): removes metadata, collapses groups, optimizes paths.
