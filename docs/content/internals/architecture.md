+++
title = "Architecture"
description = "How dodeca's host, cells, and caching layers work together"
weight = 0
+++

This document describes dodeca's architecture: how the host process orchestrates cells, tracks dependencies, and manages caching.

## Overview

Dodeca separates concerns into three layers:

```
┌─────────────────────────────────────────────────────────────┐
│                    HOST (dodeca binary)                     │
│                                                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │                      PICANTE                          │  │
│  │  • Tracks dependencies between queries                │  │
│  │  • Caches query results (memoization)                 │  │
│  │  • Knows what's stale when inputs change              │  │
│  │  • Persists cache to disk via facet-postcard          │  │
│  └───────────────────────────────────────────────────────┘  │
│                            │                                │
│  ┌────────────────────────┼────────────────────────────┐   │
│  │          CAS           │                            │   │
│  │  • Content-addressed   │  Host reads/writes         │   │
│  │  • Large blobs on disk │  Cells never touch         │   │
│  │  • Survives restarts   │                            │   │
│  └────────────────────────┼────────────────────────────┘   │
│                           │                                 │
│  ┌────────────────────────▼────────────────────────────┐   │
│  │              PROVIDER SERVICES                       │   │
│  │  • resolve_template(name) → content                 │   │
│  │  • resolve_import(path) → content                   │   │
│  │  • get_data(key) → value                            │   │
│  │  (all picante-tracked!)                             │   │
│  └────────────────────────┬────────────────────────────┘   │
└───────────────────────────┼─────────────────────────────────┘
                            │ roam RPC + SHM
┌───────────────────────────▼─────────────────────────────────┐
│                  CELLS (separate processes)                 │
│  • Pure async functions                                     │
│  • No caching knowledge                                     │
│  • Call back to host for dependencies                       │
│  • Return large blobs via shared memory                     │
└─────────────────────────────────────────────────────────────┘
```

## Design Principles

### 1. Host owns all caching

All caching decisions live in the host:

- **Picante** handles query memoization and dependency tracking
- **CAS** handles large blob storage (images, fonts, processed outputs)
- **Cells have no caching logic** — they're pure functions

This means:
- Consistent cache invalidation across all functionality
- Single source of truth for "what needs rebuilding"
- Cells stay simple and stateless

### 2. Cells call back to host for dependencies

When a cell needs additional data, it calls back to the host:

```
Host                              Cell (e.g., template renderer)
  │                                        │
  │── render(page, template_name) ────────▶│
  │                                        │
  │◀── resolve_template("base.html") ──────│
  │    (picante tracks this dependency!)   │
  │                                        │
  │── template content ───────────────────▶│
  │                                        │
  │◀── resolve_template("partials/nav") ───│
  │    (another tracked dependency!)       │
  │                                        │
  │── template content ───────────────────▶│
  │                                        │
  │◀── rendered HTML ──────────────────────│
  │                                        │
```

The magic: **cell callbacks flow through picante-tracked host APIs**. When the template cell calls `resolve_template("base.html")`, the host:

1. Looks up the template (tracked query)
2. Returns content to cell
3. Picante records the dependency: "this render depends on base.html"

If `base.html` changes later, picante knows to re-render pages that included it — even though the actual rendering happened in a cell.

### 3. Large blobs go through CAS, not picante

Picante's cache is serialized via facet-postcard. Storing large blobs there would be expensive:

- Slow serialization/deserialization
- Large cache files on disk
- Memory pressure

Instead:

| Data Type | Storage | Why |
|-----------|---------|-----|
| Text content (markdown, templates, SCSS) | Picante (inline) | Small, frequently accessed during queries |
| Binary blobs (images, fonts, PDFs) | CAS | Large, content-addressed, survives restarts |
| Processed outputs (optimized images, subsetted fonts) | CAS | Large, keyed by input hash |

The host stores only **hashes** in picante:

```rust
#[picante::input]
pub struct StaticFile {
    #[key]
    pub path: StaticPath,
    pub content_hash: ContentHash,  // 32 bytes, not megabytes
}
```

When actual content is needed, the host reads from CAS using the hash.

### 4. Shared memory for large transfers

Cells run as separate processes. Transferring large blobs (images, fonts) over RPC would be expensive.

Roam uses shared memory (SHM) for zero-copy transfers:

```
Host                              Cell
  │                                  │
  │── process_image(hash) ──────────▶│
  │                                  │
  │   (cell reads input from SHM)    │
  │   (cell writes output to SHM)    │
  │                                  │
  │◀── output_hash ──────────────────│
  │                                  │
  │   (host reads output from SHM)   │
  │   (host writes to CAS)           │
```

The host:
1. Writes input blob to SHM before calling cell
2. Cell processes in-place or writes output to SHM
3. Host reads output from SHM and stores in CAS

Cells never touch CAS directly — the host handles all persistence.

## Query Flow Example

Here's how a page render flows through the system:

```
1. Input change detected
   └─▶ SourceFile("features.md") updated in picante

2. Picante checks dependencies
   └─▶ render_page("/features") depends on this file → stale

3. Host invokes render query
   └─▶ render_page(db, "/features")
       │
       ├─▶ build_tree(db) [cached, inputs unchanged]
       │
       └─▶ call template cell via roam
           │
           ├─◀ cell calls resolve_template("page.html")
           │   └─▶ host returns template [picante tracks dependency]
           │
           ├─◀ cell calls resolve_template("base.html")
           │   └─▶ host returns template [picante tracks dependency]
           │
           ├─◀ cell calls get_data("site.title")
           │   └─▶ host returns value [picante tracks dependency]
           │
           └─▶ cell returns rendered HTML

4. Host stores result
   └─▶ picante caches rendered HTML for this route

5. Later: template changes
   └─▶ TemplateFile("base.html") updated
       └─▶ picante invalidates all pages that resolved "base.html"
```

## Cell Categories

| Cell | Inputs | Outputs | Callbacks to Host |
|------|--------|---------|-------------------|
| **Template** (gingembre) | Page data, template name | Rendered HTML | `resolve_template`, `get_data` |
| **SASS** | Entry file path | Compiled CSS | `resolve_import` |
| **Image** | Image bytes (via SHM) | Processed variants (via SHM) | None (pure transform) |
| **Fonts** | Font bytes, char set | Subsetted font (via SHM) | None (pure transform) |
| **Markdown** | Markdown text | Rendered HTML with syntax highlighting | None (pure transform) |
| **HTTP** | Request | Response | `find_content`, `eval_expression` |

"Pure transform" cells don't call back — they receive all inputs upfront and return outputs. These are the simplest to implement and reason about.

Cells with callbacks enable lazy loading and fine-grained dependency tracking, but require careful design of the provider interface.

## Cache Structure

```
.cache/
├── blobs/              # Content-addressed blob storage
│   ├── a1b2/           # Subdirectory by hash prefix
│   │   ├── a1b2c3...img    # processed image data
│   │   └── a1b2d4...ttf    # decompressed font
│   └── e5f6/
│       └── ...
├── cas.db              # Hash store (path → content hash mapping)
├── cas.version         # CAS version for cache invalidation
├── dodeca.bin          # Picante's serialized cache
├── dodeca.version      # Picante cache version
└── code-execution/     # Cached code execution results
```

Benefits:
- **Deduplication**: Same content = same hash = stored once
- **Parallel safety**: Hash-based keys prevent conflicts
- **Survives rebuilds**: Content persists even if picante cache is cleared
- **Easy cleanup**: Delete old hashes not referenced by current build

## Summary

| Concern | Owner | Why |
|---------|-------|-----|
| Dependency tracking | Picante (host) | Single source of truth for staleness |
| Query memoization | Picante (host) | Avoids redundant computation |
| Large blob storage | CAS (host) | Keeps picante cache small |
| Pure computation | Cells | Isolation, independent linking |
| Provider services | Host | Callbacks tracked by picante |

The host is the brain; cells are the muscles. Caching decisions flow through one place, making the system predictable and debuggable.
