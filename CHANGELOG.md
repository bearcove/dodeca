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
