+++
title = "Architecture"
description = "How dodeca's host, plugins, and caching layers work together"
weight = 0
+++

This document describes dodeca's architecture: how the host process orchestrates plugins, tracks dependencies, and manages caching.

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
│  │  • Large blobs on disk │  Plugins never touch       │   │
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
                            │ rapace RPC + SHM
┌───────────────────────────▼─────────────────────────────────┐
│                  PLUGINS (separate processes)               │
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
- **Plugins have no caching logic** — they're pure functions

This means:
- Consistent cache invalidation across all functionality
- Single source of truth for "what needs rebuilding"
- Plugins stay simple and stateless

### 2. Plugins call back to host for dependencies

When a plugin needs additional data, it calls back to the host:

```
Host                              Plugin (e.g., template renderer)
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

The magic: **plugin callbacks flow through picante-tracked host APIs**. When the template plugin calls `resolve_template("base.html")`, the host:

1. Looks up the template (tracked query)
2. Returns content to plugin
3. Picante records the dependency: "this render depends on base.html"

If `base.html` changes later, picante knows to re-render pages that included it — even though the actual rendering happened in a plugin.

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

Plugins run as separate processes. Transferring large blobs (images, fonts) over RPC would be expensive.

Rapace uses shared memory (SHM) for zero-copy transfers:

```
Host                              Plugin
  │                                  │
  │── process_image(hash) ──────────▶│
  │                                  │
  │   (plugin reads input from SHM)  │
  │   (plugin writes output to SHM)  │
  │                                  │
  │◀── output_hash ──────────────────│
  │                                  │
  │   (host reads output from SHM)   │
  │   (host writes to CAS)           │
```

The host:
1. Writes input blob to SHM before calling plugin
2. Plugin processes in-place or writes output to SHM
3. Host reads output from SHM and stores in CAS

Plugins never touch CAS directly — the host handles all persistence.

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
       └─▶ call template plugin via rapace
           │
           ├─◀ plugin calls resolve_template("page.html")
           │   └─▶ host returns template [picante tracks dependency]
           │
           ├─◀ plugin calls resolve_template("base.html")
           │   └─▶ host returns template [picante tracks dependency]
           │
           ├─◀ plugin calls get_data("site.title")
           │   └─▶ host returns value [picante tracks dependency]
           │
           └─▶ plugin returns rendered HTML

4. Host stores result
   └─▶ picante caches rendered HTML for this route

5. Later: template changes
   └─▶ TemplateFile("base.html") updated
       └─▶ picante invalidates all pages that resolved "base.html"
```

## Plugin Categories

| Plugin | Inputs | Outputs | Callbacks to Host |
|--------|--------|---------|-------------------|
| **Template** | Page data, template name | Rendered HTML | `resolve_template`, `get_data` |
| **SASS** | Entry file path | Compiled CSS | `resolve_import` |
| **Image** | Image bytes (via SHM) | Processed variants (via SHM) | None (pure transform) |
| **Font** | Font bytes, char set | Subsetted font (via SHM) | None (pure transform) |
| **Syntax** | Code, language | Highlighted HTML | None (pure transform) |
| **HTTP** | Request | Response | `find_content`, `eval_expression` |

"Pure transform" plugins don't call back — they receive all inputs upfront and return outputs. These are the simplest to implement and reason about.

Plugins with callbacks enable lazy loading and fine-grained dependency tracking, but require careful design of the provider interface.

## CAS Structure

Content-Addressed Storage uses content hashes as keys:

```
.cache/
├── cas/
│   ├── images/
│   │   ├── a1b2c3d4...  # processed image variants
│   │   └── e5f6g7h8...
│   ├── fonts/
│   │   ├── i9j0k1l2...  # subsetted fonts
│   │   └── m3n4o5p6...
│   └── decompressed/
│       └── q7r8s9t0...  # decompressed font (TTF from WOFF2)
└── picante.bin          # picante's serialized cache (small!)
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
| Pure computation | Plugins | Isolation, independent linking |
| Provider services | Host | Callbacks tracked by picante |

The host is the brain; plugins are the muscles. Caching decisions flow through one place, making the system predictable and debuggable.
