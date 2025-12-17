# Smart Live Reload Design

## Overview

Instead of full page reloads, dodeca sends minimal patches over WebSocket.
Server does all the diffing. Client (Rust/WASM) just applies patches.

```
┌─────────────────────────────────────────────────────────────────┐
│                         SERVER (Rust)                           │
│                                                                 │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐ │
│  │   Salsa     │───▶│  Rendered   │───▶│   Diff Engine       │ │
│  │  (cached)   │    │    HTML     │    │  (old vs new DOM)   │ │
│  └─────────────┘    └─────────────┘    └──────────┬──────────┘ │
│                                                   │             │
│                                        ┌──────────▼──────────┐ │
│                                        │   Vec<Patch>        │ │
│                                        │   (postcard)         │ │
│                                        └──────────┬──────────┘ │
└───────────────────────────────────────────────────┼─────────────┘
                                                    │ WebSocket
                                                    │ (binary)
┌───────────────────────────────────────────────────▼─────────────┐
│                        CLIENT (WASM)                            │
│                                                                 │
│  ┌─────────────────────┐    ┌─────────────────────────────────┐│
│  │  Deserialize        │───▶│  Apply patches to real DOM      ││
│  │  (postcard)          │    │  (web-sys)                      ││
│  └─────────────────────┘    └─────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

## Key Principles

1. **Server-side diffing**: Server has both old and new DOM (thanks to Salsa caching)
2. **Binary wire format**: postcard for compact, fast serialization
3. **Same code both sides**: Rust diff/patch logic shared via WASM
4. **Preserve client state**: Patches are surgical, scroll/focus/forms preserved

## What Gets Patched

### HTML Content

When markdown or templates change:
1. Server re-renders HTML
2. Diff against cached previous render
3. Send DOM patches

### CSS Styles

In development mode:
1. **Inline all CSS** via `<style>` tags (not `<link>`)
2. When CSS changes, diff the stylesheet text
3. Send text patches or full replacement
4. Client updates `<style>` element's `textContent`

Why inline? External `<link>` stylesheets can't be surgically updated.
The browser caches them by URL. Inlining gives us full control.

### Static Assets (images, fonts, etc.)

These still use cache-busting hashes. When they change:
1. Send a message with new hash
2. Client updates any `src` or `href` attributes that reference the old hash

## DOM Diffing Strategy

### Hierarchical Hash Comparison

```
Page (hash mismatch → descend)
├── <header>  hash=aaa ✓ (skip)
├── <nav>     hash=bbb ✓ (skip)
├── <article> hash=ccc ✗ (descend)
│   ├── <section id="intro">   hash=xxx ✓ (skip)
│   ├── <section id="install"> hash=eee ✗ (DIFF THIS)
│   └── <section id="usage">   hash=zzz ✓ (skip)
└── <footer>  hash=ddd ✓ (skip)
```

1. Each node has a precomputed subtree hash
2. Compare hashes top-down
3. Matching hash = skip entire subtree (O(1))
4. Mismatching hash = recurse into children
5. At small enough subtrees, run tree-edit-distance

### Tree Edit Distance

For changed subtrees, use proper tree-edit-distance algorithm:
- Zhang-Shasha or similar
- Computes minimal insert/delete/replace operations
- O(n²) but n is small (just the changed subtree)

### Child Matching

For lists of children (e.g., paragraphs in a section):
1. First try to match by `id` attribute
2. Then by stable keys (`data-key` if present)
3. Fall back to positional + tag matching
4. Compute LCS (longest common subsequence) for reordering

## Patch Operations

```rust
enum Patch {
    /// Replace node at path with new HTML
    Replace { path: NodePath, html: String },

    /// Insert HTML before the node at path
    InsertBefore { path: NodePath, html: String },

    /// Insert HTML after the node at path
    InsertAfter { path: NodePath, html: String },

    /// Append HTML as last child of node at path
    AppendChild { path: NodePath, html: String },

    /// Remove the node at path
    Remove { path: NodePath },

    /// Update text content of node at path
    SetText { path: NodePath, text: String },

    /// Set attribute on node at path
    SetAttribute { path: NodePath, name: String, value: String },

    /// Remove attribute from node at path
    RemoveAttribute { path: NodePath, name: String },

    /// Replace CSS stylesheet content
    ReplaceStyle { id: String, content: String },

    /// Update asset URL (cache bust)
    UpdateAssetUrl { selector: String, attr: String, new_url: String },
}

/// Path to a node: indices from root
/// e.g., [0, 2, 1] = root's child 0, then child 2, then child 1
struct NodePath(Vec<usize>);
```

## Wire Protocol

### WebSocket Message Types

```rust
#[derive(Serialize, Deserialize)]
enum LiveReloadMessage {
    /// Full page reload (fallback)
    Reload,

    /// DOM patches to apply
    Patches(Vec<Patch>),

    /// CSS update
    CssUpdate {
        /// Matches <style data-file="...">
        file: String,
        content: String,
    },

    /// Asset URL changed (images, fonts)
    AssetUpdate {
        old_hash: String,
        new_hash: String,
    },

    /// Connection established
    Connected {
        /// For debugging
        server_version: String,
    },
}
```

### Serialization

- **Format**: postcard (binary, fast, compact)
- **Compression**: Optional gzip for large patches
- **Framing**: WebSocket binary messages

## Client Implementation (WASM)

```rust
// lib.rs for wasm-bindgen

use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, Node};

#[wasm_bindgen]
pub fn apply_patches(data: &[u8]) -> Result<(), JsValue> {
    let patches: Vec<Patch> = postcard::deserialize(data)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let document = web_sys::window()
        .unwrap()
        .document()
        .unwrap();

    for patch in patches {
        apply_patch(&document, patch)?;
    }

    Ok(())
}

fn apply_patch(doc: &Document, patch: Patch) -> Result<(), JsValue> {
    match patch {
        Patch::SetText { path, text } => {
            let node = find_node(doc, &path)?;
            node.set_text_content(Some(&text));
        }
        Patch::Replace { path, html } => {
            let node = find_node(doc, &path)?;
            // Parse html and replace
            node.set_outer_html(&html)?;
        }
        // ... etc
    }
    Ok(())
}

fn find_node(doc: &Document, path: &NodePath) -> Result<Element, JsValue> {
    let mut current: Node = doc.body().unwrap().into();
    for &idx in &path.0 {
        current = current.child_nodes().item(idx as u32)
            .ok_or_else(|| JsValue::from_str("Node not found"))?;
    }
    current.dyn_into()
}
```

## Development Mode HTML

In dev mode, HTML includes:

```html
<!DOCTYPE html>
<html>
<head>
    <!-- CSS inlined for patchability -->
    <style data-file="css/style.css">
        /* full CSS content here */
    </style>

    <!-- WASM loader -->
    <script type="module">
        import init, { apply_patches } from '/__livereload/client.js';

        await init();

        const ws = new WebSocket(`ws://${location.host}/__livereload`);
        ws.binaryType = 'arraybuffer';

        ws.onmessage = (e) => {
            const result = apply_patches(new Uint8Array(e.data));
            if (result instanceof Error) {
                console.error('Patch failed, reloading:', result);
                location.reload();
            }
        };
    </script>
</head>
<body>
    <!-- content with data-dd-path attributes for debugging -->
</body>
</html>
```

## Production Mode

In production:
- No livereload script
- External CSS with cache-busted URLs
- No WASM client needed
- Standard static file serving

## Fallback Strategy

If patching fails for any reason:
1. Log the error
2. Send `Reload` message
3. Client does full page reload

This ensures the system is robust even if the diffing has edge cases.

## Implementation Phases

### Phase 1: CSS Hot Reload (Simple)
- [ ] Inline CSS in dev mode
- [ ] Detect CSS-only changes
- [ ] Send CSS content over WebSocket
- [ ] Client updates `<style>` textContent
- No WASM needed for this phase

### Phase 2: DOM Patching (Core)
- [ ] Implement `parse_html()` using html5ever
- [ ] Add tree-edit-distance for child matching
- [ ] postcard serialization for Patch enum
- [ ] Basic WASM client with web-sys

### Phase 3: Smart Matching
- [ ] Add `id` and `data-key` based matching
- [ ] Optimize for common patterns (paragraphs, lists)
- [ ] Add benchmarks

### Phase 4: Polish
- [ ] Compression for large patches
- [ ] Better error handling and fallback
- [ ] Debug mode with patch visualization

## Open Questions

1. **Stable node addressing**: Use paths (fragile if structure changes) or generate stable IDs during render?

2. **Handling scripts**: If `<script>` content changes, should we re-execute? Probably just reload.

3. **SVG/MathML**: Special handling needed?

4. **Shadow DOM**: If we ever use web components, patches need to pierce shadow boundaries.

## References

- [morphdom](https://github.com/patrick-steele-idem/morphdom) - DOM diffing inspiration
- [idiomorph](https://github.com/bigskysoftware/idiomorph) - htmx's morpher
- [Zhang-Shasha](https://epubs.siam.org/doi/10.1137/0218082) - Tree edit distance algorithm
- [Phoenix LiveView](https://hexdocs.pm/phoenix_live_view) - Server-side diffing inspiration
