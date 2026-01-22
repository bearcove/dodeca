# Design: Arena-based DOM with Borrowed Strings

## Overview

Replace the current recursive ownership model with:
- **indextree Arena** for tree structure (cache-friendly, flat storage)
- **Borrowed strings** from source HTML (zero-copy parsing)
- **Cow for mutations** (only allocate when actually modified)

## Key Benefits

### For Diff/Patch Use Case
```rust
// Two independent sources, two independent lifetimes
let old_html: String = load_file("old.html");
let new_html: String = load_file("new.html");

let old_doc: Document<'_> = parse(&old_html);  // Borrows from old_html
let new_doc: Document<'_> = parse(&new_html);  // Borrows from new_html

let patches = diff(&old_doc, &new_doc);  // Works! Separate lifetimes
```

### Memory Wins
- **Tag names**: "div" appears 100 times → 1 allocation from source
- **Common attributes**: "class", "id", "style" → shared from source
- **Text content**: Points directly into source HTML
- **Arena allocation**: All nodes in contiguous memory (CPU loves this)

### Performance Wins
- **Cache locality**: Arena stores nodes contiguously → better CPU cache utilization
- **Diff traversal**: Iterating over large documents is much faster
- **Live reload**: Create→diff→discard is the hot path, minimizes allocation

## Data Structures

```rust
use indextree::{Arena, NodeId};
use std::borrow::Cow;
use std::collections::HashMap;

/// Document with borrowed strings from source HTML
pub struct Document<'src> {
    /// Keep source alive
    source: &'src str,
    /// All nodes in flat arena
    arena: Arena<NodeData<'src>>,
    /// Root element (usually <html>)
    root: NodeId,
    /// DOCTYPE string (usually "html")
    doctype: Option<Cow<'src, str>>,
}

/// Node data stored in the arena
pub struct NodeData<'src> {
    pub kind: NodeKind<'src>,
    pub ns: Namespace,
}

pub enum NodeKind<'src> {
    Element(ElementData<'src>),
    Text(Cow<'src, str>),
    Comment(Cow<'src, str>),
}

pub struct ElementData<'src> {
    /// Tag name (borrowed from source, or owned if mutated)
    pub tag: Cow<'src, str>,
    /// Attributes (keys and values borrowed from source)
    pub attrs: HashMap<Cow<'src, str>, Cow<'src, str>>,
}
```

## Parsing Strategy

### Phase 1: Keep Source Alive
```rust
/// Parse HTML into arena-based DOM
pub fn parse_document(html: &str) -> Document<'_> {
    // html5ever gives us StrTendril, we need to extract borrowed &str
    let sink = ArenaSink::new(html);
    let sink = html5ever::parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_document()
}

struct ArenaSink<'src> {
    source: &'src str,
    arena: Arena<NodeData<'src>>,
    // ... other state
}
```

### Phase 2: String Borrowing from html5ever

html5ever provides `StrTendril` which is a rope-like string. We need to:
1. Check if the tendril is backed by our source string (simple substring)
2. If yes → borrow directly: `Cow::Borrowed(&source[range])`
3. If no (synthetic/merged strings) → own it: `Cow::Owned(tendril.to_string())`

```rust
impl<'src> ArenaSink<'src> {
    fn borrow_or_own(&self, tendril: &StrTendril) -> Cow<'src, str> {
        // Try to find this string within our source
        if let Some(offset) = find_substr(self.source, tendril.as_ref()) {
            let range = offset..offset + tendril.len();
            Cow::Borrowed(&self.source[range])
        } else {
            // Can't borrow - string is synthetic (merged text nodes, etc)
            Cow::Owned(tendril.to_string())
        }
    }
}

/// Fast substring search: check if needle appears in haystack
fn find_substr(haystack: &str, needle: &str) -> Option<usize> {
    // Pointer arithmetic check (very fast!)
    let hay_start = haystack.as_ptr() as usize;
    let hay_end = hay_start + haystack.len();
    let needle_ptr = needle.as_ptr() as usize;

    if needle_ptr >= hay_start && needle_ptr < hay_end {
        Some(needle_ptr - hay_start)
    } else {
        None
    }
}
```

**Note**: Most strings from html5ever WILL be substrings of the source. The parser maintains string slices. Only merged text nodes and error-recovery strings will be synthetic.

## API Design

### Navigation
```rust
impl<'src> Document<'src> {
    /// Get the <body> element if present
    pub fn body(&self) -> Option<NodeId> {
        self.root.children(&self.arena)
            .find(|&id| {
                if let NodeKind::Element(elem) = &self.arena[id].get().kind {
                    elem.tag.as_ref() == "body"
                } else {
                    false
                }
            })
    }

    /// Get immutable reference to node data
    pub fn get(&self, id: NodeId) -> &NodeData<'src> {
        self.arena[id].get()
    }

    /// Get mutable reference to node data
    pub fn get_mut(&mut self, id: NodeId) -> &mut NodeData<'src> {
        self.arena[id].get_mut()
    }

    /// Iterate children of a node
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        id.children(&self.arena)
    }
}
```

### Mutation (Cow Pattern)
```rust
impl<'src> Document<'src> {
    /// Set an attribute (converts to Owned on mutation)
    pub fn set_attr(&mut self, node: NodeId, name: &str, value: String) {
        if let NodeKind::Element(elem) = &mut self.get_mut(node).kind {
            // This converts Cow::Borrowed → Cow::Owned only when setting
            elem.attrs.insert(
                Cow::Owned(name.to_string()),
                Cow::Owned(value)
            );
        }
    }

    /// Change tag name (converts to Owned)
    pub fn set_tag(&mut self, node: NodeId, new_tag: String) {
        if let NodeKind::Element(elem) = &mut self.get_mut(node).kind {
            elem.tag = Cow::Owned(new_tag);
        }
    }
}
```

## Integration with Cinereus

**Great news**: cinereus already uses `indextree::Arena` internally! (See diff/tree.rs:9)

Current flow:
```
Document (recursive) → cinereus::Tree (arena-based) → diff → patches
```

New flow:
```
Document (already arena-based!) → adapt to cinereus traits → diff → patches
```

We can either:
1. **Implement cinereus traits directly on our arena** (cleanest)
2. **Convert to cinereus tree** (current approach, still works)

### Option 1: Direct Implementation
```rust
impl<'src> cinereus::TreeTypes for HotmealTypes<'src> {
    type Kind = NodeKind<'src>;
    type Label = NodeId;  // Use our own NodeId as label
    type Props = HtmlProps<'src>;
}

// Now diff can work directly on our Document arena!
pub fn diff_documents<'a, 'b>(
    old: &Document<'a>,
    new: &Document<'b>,
) -> Vec<Patch> {
    // No conversion needed - just run cinereus on our arena
    let (ops, matching) = cinereus::diff_trees_with_matching(
        &old.arena,
        &new.arena,
        &config,
    );
    translate_ops(ops, matching)
}
```

### Option 2: Convert to cinereus Tree (current approach)
```rust
pub fn build_cinereus_tree<'src>(doc: &Document<'src>) -> Tree<HtmlTreeTypes> {
    // Clone node data into cinereus arena
    // Cow strings stay borrowed until cloned
    let mut tree = Tree::new(/* ... */);
    // ... traverse our arena and populate cinereus arena
    tree
}
```

Option 1 is cleaner but requires more integration work.
Option 2 is safer and works with current cinereus interface.

## Serialization

```rust
impl<'src> Document<'src> {
    /// Serialize to HTML string
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        self.serialize_node(&mut out, self.root);
        out
    }

    fn serialize_node(&self, out: &mut String, id: NodeId) {
        match &self.arena[id].get().kind {
            NodeKind::Element(elem) => {
                write!(out, "<{}", elem.tag).unwrap();

                // Cow strings deref to &str automatically
                for (name, value) in &elem.attrs {
                    write!(out, " {}=\"{}\"", name, escape_attr(value)).unwrap();
                }
                out.push('>');

                // Recurse through children using indextree
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

**Cost**: Serialization still allocates a new string. But that's fine - serialization is the output, not the hot path.

## Migration Path

### Phase 1: Add Parallel Implementation
```
hotmeal/src/
  ├─ untyped_dom.rs          (current: String + recursive)
  ├─ arena_dom.rs            (new: Cow<str> + indextree)
  ├─ parser.rs               (parse to both)
  └─ diff/
      ├─ tree.rs             (current: uses untyped_dom)
      └─ arena_tree.rs       (new: uses arena_dom)
```

### Phase 2: Benchmark
```rust
#[bench]
fn bench_parse_large_doc(b: &mut Bencher) {
    let html = load_large_document();
    b.iter(|| {
        let doc = parse_document(&html);
        black_box(doc);
    });
}

#[bench]
fn bench_diff_large_docs(b: &mut Bencher) {
    let old_html = load_large_document();
    let new_html = modify_slightly(&old_html);
    b.iter(|| {
        let old = parse_document(&old_html);
        let new = parse_document(&new_html);
        let patches = diff(&old, &new);
        black_box(patches);
    });
}
```

Measure:
- Parsing time
- Memory usage (track allocations)
- Diff computation time
- Serialization time

### Phase 3: Integration
If benchmarks are favorable:
1. Make arena_dom the primary implementation
2. Keep untyped_dom for compatibility (implement From/Into)
3. Update public API to use arena_dom
4. Deprecate old API

### Phase 4: Cleanup
Remove untyped_dom once all consumers migrated.

## Open Questions

### 1. What about SVG/MathML namespaces?
**Answer**: Store `Namespace` enum in `NodeData` (same as current).
```rust
pub struct NodeData<'src> {
    pub kind: NodeKind<'src>,
    pub ns: Namespace,  // Html, Svg, MathMl
}
```

### 2. What about mutations during live reload?
**Answer**: Mutations are rare. When they happen:
- Setting attributes: `Cow::Borrowed` → `Cow::Owned` automatically
- Inserting nodes: New nodes have `Cow::Owned` strings
- Most operations (diffing, applying patches) don't mutate - they create new documents

**Hot path**: parse old → parse new → diff → discard both
**Mutation**: Only if you're building documents programmatically (less common)

### 3. html5ever merges adjacent text nodes - do we still borrow?
**Answer**: No, merged strings are `Cow::Owned`. But that's fine:
- Most text content is NOT merged (single text nodes are common)
- Tag names and attributes are NEVER merged → always borrowed
- Merged text is typically rare (multiple adjacent text nodes)

**Heuristic**: If `find_substr()` fails, we own. Simple and safe.

### 4. Lifetime complexity in API?
**Answer**: Only affects function signatures:
```rust
// Before
pub fn parse_document(html: &str) -> Document

// After
pub fn parse_document(html: &str) -> Document<'_>

// Or explicit
pub fn parse_document<'src>(html: &'src str) -> Document<'src>
```

Users don't need to think about lifetimes much - Rust infers them:
```rust
let html = load_file("doc.html");
let doc = parse_document(&html);  // Lifetime inferred
// doc cannot outlive html - compiler enforces this
```

### 5. What about cloning documents?
**Answer**: Clone converts all `Cow::Borrowed` → `Cow::Owned`:
```rust
impl<'src> Clone for Document<'src> {
    fn clone(&self) -> Self {
        // Deep clone the arena
        // All Cow<'src, str> become Cow<'static, str> (owned)
        Document {
            source: "",  // No longer needed
            arena: self.arena.clone(),  // Clones all Cow → Owned
            root: self.root,
            doctype: self.doctype.clone().map(|c| Cow::Owned(c.into_owned())),
        }
    }
}
```

Cloning is expensive (same as current), but you rarely need it. For diff/patch, you create two independent docs and discard both.

## Comparison: Before vs After

### Memory Layout - Before (Current)
```
Document {
  doctype: Option<String>  [heap]
  root: Element {           [heap]
    tag: String             [heap] "html"
    attrs: HashMap          [heap]
    children: Vec<Node>     [heap]
      ├─ Element {          [heap]
      │   tag: String       [heap] "head"
      │   children: Vec     [heap]
      │ }
      └─ Element {          [heap]
          tag: String       [heap] "body"
          children: Vec     [heap]
            ├─ Element {    [heap]
            │   tag: String [heap] "div"
            │ }
            └─ Element {    [heap]
                tag: String [heap] "div"
              }
        }
  }
}
```
**Total allocations**: 1 per Element + 1 per String + 1 per Vec
**Memory layout**: Scattered across heap (poor cache locality)

### Memory Layout - After (Arena + Borrowed)
```
Source HTML: "<!DOCTYPE html><html><head></head><body><div></div><div></div></body></html>"
             ^^^^^^^^^^^^^^^^^^^^^---^----^^^^-----^----^---^^^^^---^^^^---^^------^-----^

Document {
  source: &str                              // Points to source
  arena: Arena<NodeData> {                  // FLAT, CONTIGUOUS ARRAY
    [0]: NodeData { Element { tag: &source[22..26] "html", ... } }    // ← Points into source
    [1]: NodeData { Element { tag: &source[28..32] "head", ... } }    // ← Points into source
    [2]: NodeData { Element { tag: &source[46..50] "body", ... } }    // ← Points into source
    [3]: NodeData { Element { tag: &source[52..55] "div", ... } }     // ← Points into source
    [4]: NodeData { Element { tag: &source[64..67] "div", ... } }     // ← Points into source
  }
  doctype: Some(Cow::Borrowed(&source[9..13])) "html"                  // ← Points into source
}
```
**Total allocations**: 1 arena (all nodes contiguous) + 0 strings (borrowed)
**Memory layout**: Nodes in contiguous array (excellent cache locality)

### Diff Performance: Before vs After

**Before**: Walking tree requires pointer chasing
```
Load Element → Load tag String → Load children Vec → Load child Element → ...
[cache miss]   [cache miss]      [cache miss]       [cache miss]
```

**After**: Walking tree uses index arithmetic on contiguous array
```
Load NodeData[i] → Load NodeData[i+1] → Load NodeData[i+2] → ...
[cache hit]        [cache hit]          [cache hit]
```

**Expected speedup**: 2-5x for large documents (depends on cache size)

### API Impact

**Before**:
```rust
let doc = parse_document(html);
let body = doc.body().unwrap();
body.push_text("Hello");
```

**After**:
```rust
let doc = parse_document(html);  // Returns Document<'_>
let body_id = doc.body().unwrap();
let body = doc.get_mut(body_id);
if let NodeKind::Element(elem) = &mut body.kind {
    let text_id = doc.arena.new_node(NodeData {
        kind: NodeKind::Text(Cow::Owned("Hello".to_string())),
        ns: Namespace::Html,
    });
    body_id.append(text_id, &mut doc.arena);
}
```

**Trade-off**: API becomes more verbose (arena manipulation).
**Benefit**: Cache-friendly, zero-copy parsing, cheaper cloning.

For **mutation-heavy** use cases (building DOM programmatically), current API is better.
For **diff/patch** use cases (parse → diff → discard), arena API is much better.

## Recommendation

**Implement as opt-in alternative:**
- Keep `untyped_dom` as default (simple API, good for building DOM)
- Add `arena_dom` as opt-in (complex API, excellent for diff/patch)
- Let users choose based on use case:
  - Building DOM manually? Use `untyped_dom`
  - Hot-reloading large documents? Use `arena_dom`

```rust
// Simple API (current)
pub fn parse_document(html: &str) -> untyped_dom::Document;

// Fast API (new)
pub fn parse_document_arena(html: &str) -> arena_dom::Document<'_>;

// Choose based on needs
let doc = parse_document(html);         // Easy to use, more allocations
let doc = parse_document_arena(html);   // More verbose, fewer allocations
```

Then benchmark real workloads and decide whether to make arena the default.
