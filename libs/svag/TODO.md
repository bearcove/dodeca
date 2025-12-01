# TODO

## In Progress

- [x] Set up GitHub Actions CI
- [x] Set up release-plz for automated releases

## Up Next

- [ ] Close the compression gap with svgo (~3-4% behind)
  - [ ] More aggressive path optimizations (merge commands, convert curves)
  - [ ] Remove redundant transforms
  - [ ] Better number formatting (more aggressive trailing zero removal)
- [ ] Fetch svgo-test-suite for broader test coverage
- [ ] Publish to crates.io

## Future

- [ ] Font embedding in SVGs
  - [ ] Subset fonts to only used glyphs
  - [ ] Embed as base64 data URIs
- [ ] Dodeca integration (salsa query interface)
- [ ] Visual regression tests in CI (needs headless Chrome)

## Ideas

- [ ] WASM build for browser usage
- [ ] Streaming mode for large files
- [ ] Plugin system for custom optimizations
