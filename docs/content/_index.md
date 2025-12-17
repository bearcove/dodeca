+++
title = "dodeca"
description = "A fully incremental static site generator"
+++

dodeca is a static site generator written in Rust, with a focus on incremental builds and a development server with live reload.

In `ddc serve`, dodeca can update the browser without doing a full page reload by sending DOM patches to a small client-side script.

Under the hood it uses [Salsa](https://salsa-rs.github.io/salsa/) to recompute only the parts of the build graph that are affected by your edits.

## Start Here

- [Installation](/guide/installation/)
- [Quick Start](/guide/quick-start/)
- [Project layout](/guide/project-layout/)
- [CLI reference](/guide/cli/)
- [Configuration](/guide/configuration/)

## Notes / Current Constraints

- Some features are implemented as separate helper binaries (`ddc-cell-*`). If they are missing, related features may be unavailable.
- Platform support for the prebuilt installer is currently limited (see the installation page).

![Mountain landscape](/images/mountain.jpg)

*Photo by [Samuel Ferrara](https://unsplash.com/@samferrara) on Unsplash (CC0) — used here to demonstrate responsive image processing*
