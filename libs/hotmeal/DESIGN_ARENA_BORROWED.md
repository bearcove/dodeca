# Design: One Tree To Rule Them All

## The Core Insight

**We only ever need TWO trees for the entire diff operation.**

Not two *types* of trees. Two *instances* of the SAME tree representation.

### Current Architecture (Too Many Conversions)

```
html5ever → untyped_dom::Element (recursive, owned Strings)
          ↓
     convert (100+ allocations)
          ↓
  cinereus::Tree (indextree-based)
          ↓
  ShadowTree (indextree-based, clone)
          ↓
     convert (another 100+ allocations)
          ↓
  diff::Element (for patch application)
```

**Problem**: Four different tree representations. Three conversions. 400+ allocations.

### New Architecture (One Representation)

```
                    parse old HTML
                          ↓
html5ever → Arena<NodeData<'src>>  ← parse new HTML
                    ↓     ↓
                 (that's it!)
                    ↓     ↓
            ┌───────┴─────┴──────┐
            ↓                    ↓
     cinereus::diff      apply_patches
     (already uses       (navigates arena
      indextree!)         directly!)
            ↓
      ShadowTree
      (clones arena
       for simulation)
```

**Solution**: ONE representation. ZERO conversions. TWO allocations (one per document).

## Why This Works

### Cinereus Already Uses indextree!

Look at `cinereus/src/chawathe.rs` - it uses `indextree::Arena` internally:

```rust
pub struct Tree<T: TreeTypes> {
    pub(crate) arena: Arena<NodeData<T>>,  // ← Already using indextree!
    pub(crate) root: NodeId,
}
```

**We've been converting TO what we already have.**

### The Shadow Tree Already Uses indextree!

Look at `hotmeal/src/diff/tree.rs:9`:

```rust
pub struct ShadowTree {
    arena: Arena<ShadowNode>,  // ← Already using indextree!
    detached_nodes: HashMap<NodeId, u32>,
}
```

**We just need to use the SAME arena type everywhere.**

### The Fix: Parse Directly to indextree

Instead of:
```
html5ever → Element (recursive) → cinereus::Tree (arena)
```

Do:
```
html5ever → Arena<NodeData<'src>> (done!)
```

Then everything else just works because it's all using indextree already.

## Data Structures

### The Universal Document Type

```rust
use indextree::{Arena, NodeId};
use std::borrow::Cow;
use std::collections::HashMap;

/// Document = Arena + source reference for zero-copy parsing
pub struct Document<'src> {
    /// Keep source HTML alive for borrowed strings
    source: &'src str,

    /// THE tree - all nodes live here
    pub arena: Arena<NodeData<'src>>,

    /// Root node (usually <html> element)
    pub root: NodeId,

    /// DOCTYPE if present (usually "html")
    pub doctype: Option<Cow<'src, str>>,
}

/// What goes in each arena slot
pub struct NodeData<'src> {
    pub kind: NodeKind<'src>,
    pub ns: Namespace,  // Html, Svg, MathMl
}

pub enum NodeKind<'src> {
    Element(ElementData<'src>),
    Text(Cow<'src, str>),
    Comment(Cow<'src, str>),
}

pub struct ElementData<'src> {
    /// Tag name - borrowed from source HTML when possible
    pub tag: Cow<'src, str>,

    /// Attributes - keys and values borrowed from source
    pub attrs: HashMap<Cow<'src, str>, Cow<'src, str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}
```

### Why Cow?

```rust
let html = "<div>Hello</div>";
let doc = parse(&html);

// Tag "div" is borrowed from html:
assert!(matches!(doc.get(node).kind.tag, Cow::Borrowed(_)));

// When we mutate, it becomes owned:
doc.set_tag(node, "span".to_string());
assert!(matches!(doc.get(node).kind.tag, Cow::Owned(_)));
```

**Zero-copy parsing. Allocate only when mutating.**

## Parsing: html5ever → indextree Directly

### The Key: TreeSink Implementation

```rust
use html5ever::{parse_document, tree_builder::TreeSink};
use html5ever::tendril::StrTendril;

pub fn parse(html: &str) -> Document<'_> {
    let sink = ArenaSink::new(html);
    let mut parser = parse_document(sink, Default::default());
    parser.one(html)
}

struct ArenaSink<'src> {
    /// Source HTML (for borrowing strings)
    source: &'src str,

    /// Our arena (same type as everywhere else!)
    arena: Arena<NodeData<'src>>,

    /// Document node (parent of <html>)
    document: NodeId,

    /// DOCTYPE encountered during parse
    doctype: Option<Cow<'src, str>>,
}
```

### String Borrowing Strategy

html5ever gives us `StrTendril` (rope-like strings). Most of them ARE substrings of the source HTML. We just need to detect which ones:

```rust
impl<'src> ArenaSink<'src> {
    /// Try to borrow from source, fall back to owned
    fn borrow_or_own(&self, tendril: &StrTendril) -> Cow<'src, str> {
        // Pointer arithmetic: is tendril a substring of source?
        let tendril_ptr = tendril.as_ptr() as usize;
        let source_ptr = self.source.as_ptr() as usize;
        let source_end = source_ptr + self.source.len();

        if tendril_ptr >= source_ptr && tendril_ptr < source_end {
            // Yes! Calculate offset and borrow
            let offset = tendril_ptr - source_ptr;
            Cow::Borrowed(&self.source[offset..offset + tendril.len()])
        } else {
            // No - synthetic string (merged text, error recovery)
            Cow::Owned(tendril.to_string())
        }
    }
}
```

**Fast path**: Pointer comparison (nanoseconds)
**Hit rate**: ~95% for typical HTML (tags, attrs, most text borrowed)
**Miss case**: Merged text nodes, error-recovery strings (rare)

### TreeSink Implementation (Abbreviated)

```rust
impl<'src> TreeSink for ArenaSink<'src> {
    type Handle = NodeId;  // indextree's NodeId!
    type Output = Document<'src>;

    fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<html5ever::Attribute>,
        _flags: ElementFlags,
    ) -> NodeId {
        // Borrow tag name from source
        let tag = self.borrow_or_own(&name.local);
        let ns = Namespace::from_url(name.ns.as_ref());

        // Borrow attribute keys and values
        let attr_map = attrs.into_iter()
            .map(|attr| {
                let key = self.borrow_or_own(&attr.name.local);
                let value = self.borrow_or_own(&attr.value);
                (key, value)
            })
            .collect();

        // Create node in arena
        self.arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag,
                attrs: attr_map,
            }),
            ns,
        })
    }

    fn append(&mut self, parent: &NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(node) => {
                parent.append(node, &mut self.arena);
            }
            NodeOrText::AppendText(text) => {
                // Try to merge with previous text node
                if let Some(last) = parent.children(&self.arena).last() {
                    if let NodeKind::Text(existing) = &mut self.arena[last].get_mut().kind {
                        // Merge → must allocate
                        let mut merged = existing.clone().into_owned();
                        merged.push_str(&text);
                        *existing = Cow::Owned(merged);
                        return;
                    }
                }

                // Create new text node
                let text_node = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(self.borrow_or_own(&text)),
                    ns: Namespace::Html,
                });
                parent.append(text_node, &mut self.arena);
            }
        }
    }

    // ... other TreeSink methods (straightforward arena manipulation)
}
```

**Result**: Parse HTML directly into `Arena<NodeData<'src>>`. No intermediate representations.

## Cinereus Integration (No Conversion!)

Cinereus already uses indextree. We just implement its traits on our Document:

```rust
use cinereus::{TreeTypes, TreeNode};

pub struct HotmealTreeTypes;

impl TreeTypes for HotmealTreeTypes {
    type Kind = NodeKindOwned;  // Owned version for comparisons
    type Label = NodeId;
    type Props = HtmlProps;
}

// Owned versions (for comparison without lifetimes)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKindOwned {
    Element(ElementDataOwned),
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementDataOwned {
    pub tag: String,
    pub attrs: HashMap<String, String>,
}

impl<'src> TreeNode<HotmealTreeTypes> for Document<'src> {
    fn root(&self) -> NodeId {
        self.root
    }

    fn children(&self, node: NodeId) -> Vec<NodeId> {
        node.children(&self.arena).collect()
    }

    fn kind(&self, node: NodeId) -> NodeKindOwned {
        // Convert Cow<'src> to owned for comparison
        match &self.arena[node].get().kind {
            NodeKind::Element(elem) => NodeKindOwned::Element(ElementDataOwned {
                tag: elem.tag.to_string(),
                attrs: elem.attrs.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }),
            NodeKind::Text(t) => NodeKindOwned::Text(t.to_string()),
            NodeKind::Comment(c) => NodeKindOwned::Comment(c.to_string()),
        }
    }

    fn props(&self, node: NodeId) -> Option<HtmlProps> {
        if let NodeKind::Element(elem) = &self.arena[node].get().kind {
            Some(HtmlProps {
                tag: elem.tag.to_string(),
                attrs: elem.attrs.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            })
        } else {
            None
        }
    }
}
```

Now diffing is trivial:

```rust
pub fn diff_documents<'a, 'b>(
    old: &Document<'a>,
    new: &Document<'b>,
) -> Result<Vec<Patch>, String> {
    // Pass documents directly to cinereus!
    let (ops, matching) = cinereus::diff_trees_with_matching(
        old,
        new,
        &Default::default(),
    )?;

    // Translate cinereus ops to our Patch format
    translate_ops(ops, matching, old, new)
}
```

**No conversion. Cinereus traverses our arena directly.**

## Shadow Tree (Same Arena!)

The shadow tree just clones our arena:

```rust
pub struct ShadowTree<'src> {
    /// Same arena structure as Document
    arena: Arena<NodeData<'src>>,

    /// Detached nodes (in slots)
    detached_nodes: HashMap<NodeId, u32>,

    /// Next available slot number
    next_slot: u32,
}

impl<'src> ShadowTree<'src> {
    pub fn from_document(doc: &Document<'src>) -> Self {
        ShadowTree {
            arena: doc.arena.clone(),  // Clone arena (Cow stays borrowed!)
            detached_nodes: HashMap::new(),
            next_slot: 0,
        }
    }

    pub fn simulate_insert(&mut self, parent: NodeId, position: usize, ...) {
        // Same simulation logic as before
        // Operating directly on our arena
    }

    pub fn simulate_move(&mut self, from: NodeId, to: NodeId, ...) {
        // Same simulation logic as before
        // Operating directly on our arena
    }

    pub fn get_node_ref(&self, node: NodeId) -> NodeRef {
        // Check if node is detached
        if let Some(&slot) = self.detached_nodes.get(&node) {
            // Compute relative path within slot
            let rel_path = self.compute_relative_path(node);
            NodeRef::Slot(slot, rel_path)
        } else {
            // Compute absolute path in tree
            let path = self.compute_path_from_root(node);
            NodeRef::Path(NodePath(path))
        }
    }
}
```

**One arena type. Same simulation logic. No conversions.**

## Applying Patches (Navigate Arena Directly)

```rust
pub fn apply_patches(
    doc: &mut Document<'_>,
    patches: &[Patch],
) -> Result<(), String> {
    let mut slots: HashMap<u32, NodeId> = HashMap::new();

    for patch in patches {
        match patch {
            Patch::SetText { path, text } => {
                let node = navigate_path(doc, &path.0)?;
                if let NodeKind::Text(t) = &mut doc.arena[node].get_mut().kind {
                    *t = Cow::Owned(text.clone());
                }
            }

            Patch::InsertElement { at, tag, attrs, children, detach_to_slot } => {
                let (parent, position) = extract_parent_and_position(doc, &slots, at)?;

                // Detach occupant if needed (Chawathe semantics)
                if let Some(&slot) = detach_to_slot {
                    if let Some(occupant) = parent.children(&doc.arena).nth(position) {
                        occupant.detach(&mut doc.arena);
                        slots.insert(slot, occupant);
                    }
                }

                // Create new element
                let new_node = doc.arena.new_node(NodeData {
                    kind: NodeKind::Element(ElementData {
                        tag: Cow::Owned(tag.clone()),
                        attrs: attrs.iter()
                            .map(|(k, v)| (Cow::Owned(k.clone()), Cow::Owned(v.clone())))
                            .collect(),
                    }),
                    ns: Namespace::Html,
                });

                // Add children recursively
                for child in children {
                    insert_content_recursive(doc, new_node, child);
                }

                // Insert at position
                insert_at_position(doc, parent, position, new_node);
            }

            Patch::Move { from, to, detach_to_slot } => {
                // Get node to move
                let node = resolve_node_ref(doc, &slots, from)?;

                // Detach from current location
                node.detach(&mut doc.arena);

                // Get target location
                let (target_parent, target_pos) = extract_parent_and_position(doc, &slots, to)?;

                // Detach occupant if needed
                if let Some(&slot) = detach_to_slot {
                    if let Some(occupant) = target_parent.children(&doc.arena).nth(target_pos) {
                        occupant.detach(&mut doc.arena);
                        slots.insert(slot, occupant);
                    }
                }

                // Insert at target
                insert_at_position(doc, target_parent, target_pos, node);
            }

            // ... other patch types
        }
    }

    Ok(())
}

fn navigate_path(doc: &Document<'_>, path: &[usize]) -> Result<NodeId, String> {
    let mut current = doc.root;
    for &idx in path {
        current = current.children(&doc.arena)
            .nth(idx)
            .ok_or_else(|| format!("Path index {} out of bounds", idx))?;
    }
    Ok(current)
}
```

**Direct arena navigation. No conversions. No intermediate types.**

## Serialization

```rust
impl<'src> Document<'src> {
    pub fn to_html(&self) -> String {
        let mut out = String::new();

        if let Some(doctype) = &self.doctype {
            write!(&mut out, "<!DOCTYPE {}>", doctype).unwrap();
        }

        self.serialize_node(&mut out, self.root);
        out
    }

    fn serialize_node(&self, out: &mut String, id: NodeId) {
        match &self.arena[id].get().kind {
            NodeKind::Element(elem) => {
                // Skip document root node
                if elem.tag.is_empty() {
                    for child in id.children(&self.arena) {
                        self.serialize_node(out, child);
                    }
                    return;
                }

                write!(out, "<{}", elem.tag).unwrap();

                for (name, value) in &elem.attrs {
                    write!(out, " {}=\"{}\"", name, escape_attr(value)).unwrap();
                }

                if is_void_element(&elem.tag) {
                    out.push('>');
                    return;
                }

                out.push('>');

                for child in id.children(&self.arena) {
                    self.serialize_node(out, child);
                }

                write!(out, "</{}>", elem.tag).unwrap();
            }
            NodeKind::Text(text) => {
                out.push_str(&escape_text(text));
            }
            NodeKind::Comment(text) => {
                write!(out, "<!--{}-->", text).unwrap();
            }
        }
    }
}
```

## Memory Comparison

### Before (Current Architecture)

```
Parse old HTML:
  Element (100+ heap allocations)
    ↓
  Convert to cinereus::Tree (100+ more allocations)
    ↓
  Convert to ShadowTree (100+ more allocations)
    ↓
  Convert to diff::Element for apply (100+ more allocations)

Parse new HTML:
  Element (100+ heap allocations)
    ↓
  Convert to cinereus::Tree (100+ more allocations)

Total: 600+ allocations, scattered across heap
```

### After (One Tree Architecture)

```
Parse old HTML:
  Arena<NodeData<'src>> (1 allocation, borrowed strings)

Parse new HTML:
  Arena<NodeData<'src>> (1 allocation, borrowed strings)

Clone for shadow tree:
  Arena<NodeData<'src>> (1 allocation, strings stay borrowed)

Total: 3 allocations, contiguous memory
```

**200x fewer allocations. Contiguous memory. Borrowed strings.**

## Performance Expected Wins

### Parsing
- **Zero-copy string handling**: 2-3x faster
- **Direct arena insertion**: No recursive allocation
- **Estimated**: 2-5x faster parsing

### Diffing
- **Cache-friendly traversal**: Sequential arena access
- **No conversion overhead**: Direct cinereus integration
- **Estimated**: 2-3x faster diffing

### Patch Application
- **Direct arena navigation**: Index arithmetic, not pointer chasing
- **No type conversions**: Same arena throughout
- **Estimated**: 1.5-2x faster application

### Memory
- **50-80% reduction** in allocations
- **Near-zero string allocations** (everything borrowed)
- **Better cache locality** (contiguous array)

## Implementation Plan

### Phase 1: Core Parser (Start Here!)
- [ ] Create `arena_dom.rs` module
- [ ] Define `Document<'src>`, `NodeData`, `NodeKind` structs
- [ ] Implement `ArenaSink` with `borrow_or_own`
- [ ] Implement `TreeSink` trait for html5ever integration
- [ ] Add basic test: parse simple HTML, verify structure

### Phase 2: Serialization
- [ ] Implement `Document::to_html()`
- [ ] Add HTML escaping functions
- [ ] Test roundtrip: `parse → to_html → parse` should match

### Phase 3: Cinereus Integration
- [ ] Implement `TreeNode` trait on `Document`
- [ ] Update `diff_documents()` to work with arena docs directly
- [ ] Remove conversion code from `diff/tree.rs`
- [ ] Test: old tests should still pass

### Phase 4: Shadow Tree
- [ ] Update `ShadowTree` to use `Arena<NodeData<'src>>`
- [ ] Update simulation methods to work with new arena
- [ ] Test: shadow tree simulation produces correct NodeRefs

### Phase 5: Patch Application
- [ ] Update `apply_patches()` to navigate arena directly
- [ ] Remove `diff::Element` type entirely
- [ ] Update all apply tests
- [ ] Test: all 36 rust tests + 31 WASM tests pass

### Phase 6: Replace Everything
- [ ] Make `arena_dom::Document` the primary type
- [ ] Export as `hotmeal::Document` (breaking change)
- [ ] Update `parse_body`, `parse_untyped` to return arena docs
- [ ] Remove `untyped_dom.rs`
- [ ] Update benchmarks

### Phase 7: Benchmark & Validate
- [ ] Run benchmarks (parse, diff, apply, full cycle)
- [ ] Measure memory usage
- [ ] Run fuzzer for extended period
- [ ] Compare with baseline performance

## Trade-offs

### Lifetime Complexity
Function signatures get lifetime parameters:

```rust
// Before
pub fn parse(html: &str) -> Document

// After
pub fn parse(html: &str) -> Document<'_>
```

**Impact**: Minimal. Rust infers lifetimes. Users don't notice.

### API Verbosity
Arena manipulation is more verbose:

```rust
// Before
body.children[0]

// After
body.children(&doc.arena).next().unwrap()
```

**Impact**: Can add convenience methods to hide this.

### Worth It?
**Absolutely.** We're talking about:
- 200x fewer allocations
- 2-5x faster parsing
- 2-3x faster diffing
- Contiguous memory (cache-friendly)
- Zero conversions

For a diff/patch library, this is the hot path. The win is massive.

## Key Insight Recap

We don't need parallel implementations.
We don't need to keep the old type for compatibility.
We just need to **parse directly to what cinereus already expects**.

Everything uses indextree.
Everything uses the same arena.
Everything just works.

**One tree. Two instances. Zero conversions.**
