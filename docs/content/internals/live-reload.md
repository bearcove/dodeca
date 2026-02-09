+++
title = "Live Reload"
weight = 22
+++

Instead of refreshing the page, dodeca patches the DOM directly. You keep your scroll position, form state, and focus.

```mermaid
sequenceDiagram
    participant FS as File System
    participant P as Picante
    participant LR as LiveReloadServer
    participant WS as WebSocket
    participant B as Browser (WASM client)

    FS->>P: file changed
    P->>P: recompute affected queries
    P-->>LR: new HTML for route
    LR->>LR: diff old HTML vs new HTML
    LR->>LR: compute minimal patches
    LR->>WS: send patches
    WS->>B: patch messages
    B->>B: apply DOM patches in-place
    Note over B: scroll position, form state,<br/>and focus preserved
```

When a file changes, Picante rebuilds only what's affected. The server diffs the old and new HTML using [hotmeal](https://github.com/bearcove/hotmeal)'s tree diffing algorithm, computes minimal edit operations, and sends them over WebSocket. A small WASM client applies the patches—replacing nodes, updating text, adding or removing attributes—without touching anything that didn't change.

```rust
enum Patch {
    Replace { path: NodePath, html: String },
    InsertBefore { path: NodePath, html: String },
    Remove { path: NodePath },
    SetText { path: NodePath, text: String },
    SetAttribute { path: NodePath, name: String, value: String },
    // ...
}
```

CSS is simpler: we just update the `<link>` href to the new cache-busted URL and let the browser fetch it.

If patching fails (rare), we fall back to a full reload.
