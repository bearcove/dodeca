# bearmark - Markdown Rendering Library

## Overview

`bearmark` is a shared markdown rendering library in the dodeca workspace, consumed by:
- **cell-markdown** (dodeca) - for static site generation
- **tracey** - for spec rendering in the dashboard

## Design Goals

1. **Pluggable code block handlers** - DI pattern for syntax highlighting, diagrams
2. **Frontmatter support** - TOML (`+++`) and YAML (`---`)
3. **Heading extraction** - with slug generation for TOC
4. **Req definitions** - `r[req.id]` syntax for spec traceability
5. **No heavy dependencies** - consumers bring their own handlers

## Core API

See rustdoc
