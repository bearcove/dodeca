# Integration Plan: Arena DOM with Existing Codebase

## Current Architecture

```
┌─────────────┐
│  html5ever  │  (external parser)
│ (StrTendril)│
└──────┬──────┘
       │ parse
       ▼
┌─────────────────┐
│  untyped_dom    │  (String, recursive)
│  Document       │
│  Element        │
│  Node           │
└──────┬──────────┘
       │ convert
       ▼
┌─────────────────┐
│  diff::apply    │  (simplified DOM)
│  Element        │
│  Content        │
└──────┬──────────┘
       │ build_tree
       ▼
┌─────────────────┐
│  diff::tree     │  (cinereus integration)
│  HtmlTreeTypes  │
│  HtmlNodeKind   │
│  HtmlProps      │
└──────┬──────────┘
       │ diff_elements
       ▼
┌─────────────────┐
│    cinereus     │  (external diff library)
│ Tree<Types>     │
│ (indextree)     │
└──────┬──────────┘
       │ EditOp
       ▼
┌─────────────────┐
│     Patch       │  (hotmeal patches)
│  NodeRef        │
│  NodePath       │
└─────────────────┘
```

## Proposed Architecture

```
┌─────────────┐
│  html5ever  │
│ (StrTendril)│
└──────┬──────┘
       │ parse (borrow strings)
       ▼
┌─────────────────┐
│   arena_dom     │  (NEW: Cow<'src, str>, indextree)
│  Document<'src> │
│  NodeData<'src> │
│  NodeKind<'src> │
└──────┬──────────┘
       │ already indexed!
       ▼
┌─────────────────┐
│  cinereus       │  (DIRECT: arena is already indexed)
│ diff_trees()    │
└──────┬──────────┘
       │ EditOp
       ▼
┌─────────────────┐
│     Patch       │  (existing patch types)
└─────────────────┘
```

**Key insight**: By using `indextree::Arena` from the start, we eliminate one conversion step!

## Integration Strategy: Three Phases

### Phase 1: Side-by-Side Implementation

Add new modules alongside existing ones:

```
hotmeal/src/
  ├─ untyped_dom.rs           (keep existing)
  ├─ arena_dom.rs             (NEW: arena-based types)
  ├─ parser.rs                (modify: add parse_arena)
  ├─ serialize.rs             (keep for untyped_dom)
  ├─ arena_serialize.rs       (NEW: serialize arena_dom)
  └─ diff/
      ├─ mod.rs               (keep existing)
      ├─ apply.rs             (keep existing)
      ├─ tree.rs              (keep existing)
      ├─ arena_diff.rs        (NEW: diff for arena_dom)
      └─ translate.rs
```

### Phase 2: Parser Integration (parser.rs)

#### Current Approach
```rust
// parser.rs currently
impl TreeSink for HtmlSink {
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, ..) -> NodeHandle {
        let attrs = attrs
            .into_iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            //           ^^^^^^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^^^^^^
            //           Allocates new String   Allocates new String
            .collect();
        // ...
    }
}
```

#### New Approach
```rust
// parser.rs - new ArenaSink
struct ArenaSink<'src> {
    source: &'src str,
    arena: Arena<NodeData<'src>>,
    // ... other state
}

impl<'src> TreeSink for ArenaSink<'src> {
    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, ..) -> NodeId {
        let attrs = attrs
            .into_iter()
            .map(|a| {
                (
                    borrow_or_own(self.source, a.name.local.as_ref()),
                    borrow_or_own(self.source, a.value.as_ref()),
                )
            })
            .collect();

        self.arena.new_node(NodeData {
            kind: NodeKind::Element(ElementData {
                tag: borrow_or_own(self.source, name.local.as_ref()),
                attrs,
            }),
            ns: map_namespace(&name.ns),
        })
    }

    fn append(&self, parent: &NodeId, child: NodeOrText<NodeId>) {
        match child {
            NodeOrText::AppendNode(child_id) => {
                parent.append(child_id, &mut self.arena);
            }
            NodeOrText::AppendText(text) => {
                let text_id = self.arena.new_node(NodeData {
                    kind: NodeKind::Text(borrow_or_own(self.source, &text)),
                    ns: Namespace::Html,
                });
                parent.append(text_id, &mut self.arena);
            }
        }
    }
}

/// Try to borrow string from source, otherwise own it
fn borrow_or_own<'src>(source: &'src str, s: &str) -> Cow<'src, str> {
    let src_start = source.as_ptr() as usize;
    let src_end = src_start + source.len();
    let s_ptr = s.as_ptr() as usize;

    if s_ptr >= src_start && s_ptr + s.len() <= src_end {
        let offset = s_ptr - src_start;
        Cow::Borrowed(&source[offset..offset + s.len()])
    } else {
        Cow::Owned(s.to_string())
    }
}
```

**Key points:**
1. html5ever provides `StrTendril`, which is often a substring of source
2. We check pointer addresses to detect substrings (fast!)
3. Only allocate when html5ever synthesized the string (merged text, error recovery)
4. Most strings (tag names, attributes) will be borrowed

### Phase 3: Cinereus Integration

#### Option A: Implement TreeTypes Directly (Best)

```rust
// arena_dom.rs
pub struct ArenaTreeTypes<'src>(std::marker::PhantomData<&'src ()>);

impl<'src> cinereus::TreeTypes for ArenaTreeTypes<'src> {
    type Kind = NodeKind<'src>;
    type Label = NodeId;  // Use indextree NodeId as label
    type Props = ArenaProps<'src>;
}

pub struct ArenaProps<'src> {
    attrs: HashMap<Cow<'src, str>, Cow<'src, str>>,
    text: Option<Cow<'src, str>>,
}

impl<'src> cinereus::Properties for ArenaProps<'src> {
    type Key = Cow<'src, str>;
    type Value = Cow<'src, str>;

    fn similarity(&self, other: &Self) -> f64 {
        // Same as current HtmlProps::similarity
        // ...
    }

    fn diff(&self, other: &Self) -> Vec<PropertyChange<Self::Key, Self::Value>> {
        // Same as current HtmlProps::diff
        // ...
    }
}

// Now diff directly on our arena!
pub fn diff_documents<'a, 'b>(
    old: &Document<'a>,
    new: &Document<'b>,
) -> Result<Vec<Patch>, String> {
    // Convert our arena to cinereus Tree (cheap, mostly wrapping)
    let tree_a = wrap_arena_as_tree(&old.arena, old.root);
    let tree_b = wrap_arena_as_tree(&new.arena, new.root);

    let config = MatchingConfig {
        min_height: 0,
        ..Default::default()
    };

    let (ops, matching) = diff_trees_with_matching(&tree_a, &tree_b, &config);
    convert_ops_to_patches(ops, matching, &tree_a, &tree_b)
}

/// Wrap our arena as a cinereus Tree (zero-copy view)
fn wrap_arena_as_tree<'src>(
    arena: &Arena<NodeData<'src>>,
    root: NodeId,
) -> cinereus::Tree<ArenaTreeTypes<'src>> {
    // cinereus also uses indextree::Arena internally
    // We can potentially share the arena or convert cheaply
    // ...
}
```

#### Option B: Convert to Cinereus Tree (Current Approach)

Keep the current `diff::tree::build_tree` approach, but adapt it to work with `arena_dom`:

```rust
// diff/arena_diff.rs
pub fn build_tree<'src>(doc: &Document<'src>) -> Tree<HtmlTreeTypes> {
    let mut tree = Tree::new(/* root data */);

    // Recursively traverse our arena and build cinereus tree
    fn add_node(
        tree: &mut Tree<HtmlTreeTypes>,
        parent: cinereus::NodeId,
        doc: &Document,
        node_id: NodeId,
        path: Vec<usize>,
    ) {
        if let Some(node) = doc.get(node_id) {
            match &node.kind {
                NodeKind::Element(elem) => {
                    let data = NodeData {
                        hash: NodeHash(0),
                        kind: HtmlNodeKind::Element(elem.tag.to_string()),
                        label: Some(NodePath(path.clone())),
                        properties: HtmlProps {
                            attrs: elem.attrs.iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect(),
                            text: None,
                        },
                    };
                    let cinereus_id = tree.add_child(parent, data);

                    // Add children
                    for (i, child_id) in doc.children(node_id).enumerate() {
                        let mut child_path = path.clone();
                        child_path.push(i);
                        add_node(tree, cinereus_id, doc, child_id, child_path);
                    }
                }
                NodeKind::Text(text) => {
                    let data = NodeData {
                        hash: NodeHash(0),
                        kind: HtmlNodeKind::Text,
                        label: Some(NodePath(path)),
                        properties: HtmlProps {
                            attrs: HashMap::new(),
                            text: Some(text.to_string()),
                        },
                    };
                    tree.add_child(parent, data);
                }
                NodeKind::Comment(_) => {
                    // Skip comments (current behavior)
                }
            }
        }
    }

    add_node(&mut tree, tree.root, doc, doc.root, vec![]);
    recompute_hashes(&mut tree);
    tree
}
```

**Trade-off**: Option B requires converting strings from `Cow<'src, str>` → `String`, but keeps cinereus integration simple.

**Recommendation**: Start with Option B (simpler), optimize to Option A later if needed.

### Phase 4: Public API

Expose both implementations:

```rust
// lib.rs

/// Parse HTML into recursive DOM (current API)
pub fn parse_document(html: &str) -> untyped_dom::Document {
    parser::parse_document(html)
}

/// Parse HTML into arena-based DOM (new API)
pub fn parse_document_arena(html: &str) -> arena_dom::Document<'_> {
    parser::parse_document_arena(html)
}

/// Diff two documents (works with both types)
pub fn diff(old: &untyped_dom::Document, new: &untyped_dom::Document) -> Vec<Patch> {
    // Current implementation
    diff::diff_documents(old, new)
}

/// Diff two arena documents (optimized path)
pub fn diff_arena<'a, 'b>(
    old: &arena_dom::Document<'a>,
    new: &arena_dom::Document<'b>,
) -> Vec<Patch> {
    // New implementation
    diff::arena_diff::diff_documents(old, new)
}
```

## Migration Guide for Users

### Before (Current API)
```rust
use hotmeal::{parse_document, diff};

fn hot_reload(old_html: &str, new_html: &str) -> Vec<Patch> {
    let old = parse_document(old_html);
    let new = parse_document(new_html);
    diff(&old, &new)
}
```

### After (Arena API)
```rust
use hotmeal::{parse_document_arena, diff_arena};

fn hot_reload(old_html: &str, new_html: &str) -> Vec<Patch> {
    let old = parse_document_arena(old_html);
    let new = parse_document_arena(new_html);
    diff_arena(&old, &new)
}
```

**Changes**: Just add `_arena` suffix to functions. That's it!

## Benchmarking Plan

### Metrics to Measure

1. **Parse time**: How long to build the DOM
2. **Memory usage**: Bytes allocated during parsing
3. **Diff time**: How long to compute patches
4. **Serialization time**: How long to render HTML
5. **Cache performance**: L1/L2 cache hit rates (perf stat on Linux)

### Test Cases

```rust
// Small document (1 KB)
let small = r#"<html><body><div>Hello</div></body></html>"#;

// Medium document (100 KB)
let medium = load_file("tests/fixtures/medium.html");

// Large document (1 MB)
let large = load_file("tests/fixtures/large.html");

// Very large document (10 MB)
let very_large = load_file("tests/fixtures/very_large.html");

#[bench]
fn bench_parse_arena_small(b: &mut Bencher) {
    b.iter(|| {
        let doc = parse_document_arena(small);
        black_box(doc);
    });
}

#[bench]
fn bench_parse_recursive_small(b: &mut Bencher) {
    b.iter(|| {
        let doc = parse_document(small);
        black_box(doc);
    });
}

// Repeat for medium, large, very_large
// Repeat for diff, serialize operations

#[bench]
fn bench_diff_arena_large(b: &mut Bencher) {
    let old = parse_document_arena(large);
    let new = parse_document_arena(&modify_slightly(large));
    b.iter(|| {
        let patches = diff_arena(&old, &new);
        black_box(patches);
    });
}
```

### Expected Results

**Predictions** (based on typical arena performance):

| Metric               | Recursive (current) | Arena (new) | Speedup |
|---------------------|---------------------|-------------|---------|
| Parse small (1KB)   | 50 µs              | 40 µs       | 1.25x   |
| Parse large (1MB)   | 50 ms              | 20 ms       | 2.5x    |
| Memory (1MB doc)    | 5 MB               | 2 MB        | 2.5x    |
| Diff large          | 30 ms              | 10 ms       | 3x      |
| Serialize           | 10 ms              | 10 ms       | 1x      |

**Why**: Arena wins on large documents due to:
- Fewer allocations (arena bulk allocates)
- Better cache locality (contiguous memory)
- Borrowed strings (zero-copy)

## Risks and Mitigations

### Risk 1: Lifetime complexity in public API

**Problem**: Users need to understand `Document<'_>` lifetime

**Mitigation**:
```rust
// Bad: Lifetime error
fn process(html: &str) -> Document<'_> {
    parse_document_arena(html)
}  // html dropped here!

// Good: Keep source alive
fn process(html: &String) -> Document<'_> {
    parse_document_arena(html)
}

// Better: Document owns source
pub struct OwnedDocument {
    _source: String,
    doc: Document<'static>,  // Pretend it's static (we own source)
}
```

Provide helper for owned documents if lifetime management is too complex.

### Risk 2: Cow doesn't actually borrow

**Problem**: html5ever might not provide substrings of source

**Mitigation**:
- Add instrumentation: Count borrowed vs owned in tests
- If borrowing rate < 50%, reconsider approach
- Fallback: Use `smol_str` instead (still better than `String`)

### Risk 3: Performance doesn't improve

**Problem**: Benchmarks show no significant speedup

**Mitigation**:
- Still worth it for memory reduction alone
- Make arena opt-in, not default
- Document when to use each approach

### Risk 4: cinereus integration breaks

**Problem**: Converting to/from cinereus Tree is complex

**Mitigation**:
- Start with Option B (convert to cinereus Tree)
- Keep existing test suite passing
- Property testing: `apply(diff(A, B)) == B`

## Success Criteria

Proceed with full integration if:
1. ✅ Parsing is ≥1.5x faster on large documents (>100KB)
2. ✅ Memory usage is ≥2x better on large documents
3. ✅ All existing tests pass
4. ✅ Borrowing rate ≥60% (most strings borrowed, not owned)
5. ✅ API is not significantly more complex (just `_arena` suffix)

If any criterion fails, keep as opt-in alternative instead of replacing default.

## Timeline Estimate

- **Week 1**: Implement `arena_dom` module
- **Week 2**: Implement `ArenaSink` parser
- **Week 3**: Implement cinereus integration
- **Week 4**: Benchmarking and optimization
- **Week 5**: Documentation and tests
- **Week 6**: Real-world testing with large documents

Total: ~6 weeks for full implementation and validation.

## Next Steps

1. **Create prototype branch**: `git checkout -b feat/arena-dom`
2. **Add arena_dom.rs**: Basic types with lifetimes
3. **Add string borrowing**: Test with real HTML samples
4. **Measure borrowing rate**: Count Borrowed vs Owned
5. **Decision point**: Continue if borrowing rate > 60%
6. **Full implementation**: Parser, diff, serialize
7. **Benchmarks**: Validate performance gains
8. **Final decision**: Make default vs keep opt-in
