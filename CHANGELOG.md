## 0.2.10 (2025-12-03)

### Fixes

- use bash shell for all WASM build steps (Windows PATH issue)
- add cargo bin to PATH for wasm-bindgen

## 0.2.9 (2025-12-03)

### Fixes

- use cargo metadata for cross-platform wasm-bindgen version detection

## 0.2.8 (2025-12-03)

### Fixes

- pin wasm-bindgen-cli to match lockfile version
- remove committed pkg files, auto-detect wasm-bindgen version from lockfile

## 0.2.7 (2025-12-03)

### Fixes

- use wasm-bindgen-cli directly instead of wasm-pack

## 0.2.6 (2025-12-03)

### Fixes

- use cargo-binstall for faster wasm-pack install

## 0.2.5 (2025-12-03)

### Fixes

- use cargo install wasm-pack for ARM64 compatibility

## 0.2.4 (2025-12-03)

### Fixes

- use jetli/wasm-pack-action for cross-platform wasm-pack install

## 0.2.3 (2025-12-03)

### Fixes

- discover plugins dynamically instead of hardcoded list
- add wasm-pack pre-build step for cargo-dist releases

## 0.2.2 (2025-12-03)

### Fixes

- use facet crate attribute in plugcard macro

## 0.2.1 (2025-12-03)

### Fixes

- add rust-toolchain.toml for Rust 1.91 (#62)

## 0.2.0 (2025-12-03)

### Features

- migrate CLI parsing from clap to facet-args (#20)
- migrate plugcard to facet-postcard (#22)
- WOFF2 font subsetting + data files for templates (#23)
- add page ancestors metadata for breadcrumbs
- expose last_updated timestamp to templates (#27)
- add continue/break support and exclude root from ancestors (#46)
- implement CSS hot-reload (#47)
- integrate search indexing in build mode and store blobs on disk (#48)
- automatically add .cache to nearest .gitignore (#50)
- add rate limiting for external link checker (#51)
- add benchmarks for critical paths (#53)
- modularize minify, svgo, and sass into plugins (#55)
- modularize CSS and JS processing into plugins (#56)

### Fixes

- inject id attributes into HTML headings for anchor links
- remove trailing slashes from @/ internal link resolution
- set page=None in section context + expand template docs (#34)
- use tempfile crate for test fixture isolation (#57)
- detect file moves on macOS (closes #9, #10) (#58)
