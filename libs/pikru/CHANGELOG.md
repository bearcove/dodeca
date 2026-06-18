# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.1](https://github.com/bearcove/pikru/compare/pikru-v1.2.0...pikru-v1.2.1) - 2026-05-19

### Fixed

- resolve clippy collapsible_match and unnecessary_sort_by lints
- add ..Default::default() to RenderOptions doctests
- implement 10 missing grammar productions from pikchr spec

### Other

- untrack vim swap file, gitignore *.swp
- Add `explicit_size` option to `RenderOptions` ([#6](https://github.com/bearcove/pikru/pull/6))
- Upgrade to facet 0.46
- grammar gap .pikchr files
