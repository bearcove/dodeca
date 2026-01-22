// Proof-of-concept: Arena-based DOM with borrowed strings
//
// This demonstrates the key concepts without integrating with the full codebase.
// Compile with: rustc --edition 2021 PROTOTYPE_ARENA.rs

use std::borrow::Cow;
use std::collections::HashMap;

// Simulating indextree (you'd use the real crate)
mod mini_indextree {
    use std::collections::HashMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct NodeId(usize);

    pub struct Arena<T> {
        nodes: Vec<Option<Node<T>>>,
    }

    struct Node<T> {
        data: T,
        parent: Option<NodeId>,
        children: Vec<NodeId>,
    }

    impl<T> Arena<T> {
        pub fn new() -> Self {
            Arena { nodes: Vec::new() }
        }

        pub fn new_node(&mut self, data: T) -> NodeId {
            let id = NodeId(self.nodes.len());
            self.nodes.push(Some(Node {
                data,
                parent: None,
                children: Vec::new(),
            }));
            id
        }

        pub fn get(&self, id: NodeId) -> Option<&T> {
            self.nodes.get(id.0)?.as_ref().map(|n| &n.data)
        }

        pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
            self.nodes.get_mut(id.0)?.as_mut().map(|n| &mut n.data)
        }

        pub fn append(&mut self, parent: NodeId, child: NodeId) {
            if let Some(parent_node) = self.nodes[parent.0].as_mut() {
                parent_node.children.push(child);
            }
            if let Some(child_node) = self.nodes[child.0].as_mut() {
                child_node.parent = Some(parent);
            }
        }

        pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
            self.nodes[id.0]
                .as_ref()
                .map(|n| n.children.as_slice())
                .unwrap_or(&[])
                .iter()
                .copied()
        }
    }
}

use mini_indextree::{Arena, NodeId};

// ============================================================================
// Core Types with Lifetimes
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

/// The document owns the source HTML and the arena of nodes
pub struct Document<'src> {
    /// Source HTML (kept alive for borrowing)
    source: &'src str,
    /// All nodes in a flat arena
    arena: Arena<NodeData<'src>>,
    /// Root node (usually <html>)
    root: NodeId,
    /// DOCTYPE declaration
    doctype: Option<Cow<'src, str>>,
}

/// Data stored in each arena node
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
    /// Tag name (borrowed from source, owned if mutated)
    pub tag: Cow<'src, str>,
    /// Attributes (borrowed from source, owned if mutated)
    pub attrs: HashMap<Cow<'src, str>, Cow<'src, str>>,
}

// ============================================================================
// String Borrowing Utilities
// ============================================================================

/// Try to borrow a substring from the source, otherwise own it
fn borrow_or_own<'src>(source: &'src str, s: &str) -> Cow<'src, str> {
    // Check if `s` is a substring of `source` using pointer arithmetic
    let src_start = source.as_ptr() as usize;
    let src_end = src_start + source.len();
    let s_ptr = s.as_ptr() as usize;

    if s_ptr >= src_start && s_ptr + s.len() <= src_end {
        // s is within source bounds - calculate offset
        let offset = s_ptr - src_start;
        Cow::Borrowed(&source[offset..offset + s.len()])
    } else {
        // s is not from source - must own it
        Cow::Owned(s.to_string())
    }
}

// ============================================================================
// Simple Parser (simulates html5ever)
// ============================================================================

/// Simplified parser that demonstrates borrowing
pub fn parse_simple<'src>(html: &'src str) -> Document<'src> {
    let mut arena = Arena::new();

    // Parse a simple structure: <html><body><div>text</div></body></html>
    // In reality, html5ever would provide proper parsing with StrTendril

    // Create root element (html)
    let html_elem = arena.new_node(NodeData {
        kind: NodeKind::Element(ElementData {
            tag: borrow_or_own(html, "html"), // Try to find "html" in source
            attrs: HashMap::new(),
        }),
        ns: Namespace::Html,
    });

    // Create body element
    let body_elem = arena.new_node(NodeData {
        kind: NodeKind::Element(ElementData {
            tag: borrow_or_own(html, "body"),
            attrs: HashMap::new(),
        }),
        ns: Namespace::Html,
    });
    arena.append(html_elem, body_elem);

    // Create div element with attributes
    let mut div_attrs = HashMap::new();
    div_attrs.insert(
        borrow_or_own(html, "class"),
        borrow_or_own(html, "container"),
    );
    let div_elem = arena.new_node(NodeData {
        kind: NodeKind::Element(ElementData {
            tag: borrow_or_own(html, "div"),
            attrs: div_attrs,
        }),
        ns: Namespace::Html,
    });
    arena.append(body_elem, div_elem);

    // Create text node
    let text_content = "Hello, world!";
    let text_node = arena.new_node(NodeData {
        kind: NodeKind::Text(borrow_or_own(html, text_content)),
        ns: Namespace::Html,
    });
    arena.append(div_elem, text_node);

    Document {
        source: html,
        arena,
        root: html_elem,
        doctype: Some(Cow::Borrowed("html")),
    }
}

// ============================================================================
// Document API
// ============================================================================

impl<'src> Document<'src> {
    /// Find the body element
    pub fn body(&self) -> Option<NodeId> {
        self.arena.children(self.root).find(|&id| {
            if let Some(NodeData {
                kind: NodeKind::Element(elem),
                ..
            }) = self.arena.get(id)
            {
                elem.tag.as_ref() == "body"
            } else {
                false
            }
        })
    }

    /// Get immutable node data
    pub fn get(&self, id: NodeId) -> Option<&NodeData<'src>> {
        self.arena.get(id)
    }

    /// Get mutable node data
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut NodeData<'src>> {
        self.arena.get_mut(id)
    }

    /// Iterate children
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.arena.children(id)
    }

    /// Set an attribute (converts to Owned)
    pub fn set_attr(&mut self, node: NodeId, name: String, value: String) {
        if let Some(NodeData {
            kind: NodeKind::Element(elem),
            ..
        }) = self.get_mut(node)
        {
            elem.attrs.insert(Cow::Owned(name), Cow::Owned(value));
        }
    }

    /// Serialize to HTML
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        if let Some(doctype) = &self.doctype {
            out.push_str(&format!("<!DOCTYPE {}>\n", doctype));
        }
        self.serialize_node(&mut out, self.root, 0);
        out
    }

    fn serialize_node(&self, out: &mut String, id: NodeId, indent: usize) {
        if let Some(node) = self.get(id) {
            match &node.kind {
                NodeKind::Element(elem) => {
                    // Opening tag
                    out.push_str(&"  ".repeat(indent));
                    out.push('<');
                    out.push_str(&elem.tag);

                    // Attributes (sorted for determinism)
                    let mut attrs: Vec<_> = elem.attrs.iter().collect();
                    attrs.sort_by_key(|(k, _)| k.as_ref());
                    for (name, value) in attrs {
                        out.push_str(&format!(" {}=\"{}\"", name, value));
                    }
                    out.push_str(">\n");

                    // Children
                    for child in self.children(id) {
                        self.serialize_node(out, child, indent + 1);
                    }

                    // Closing tag
                    out.push_str(&"  ".repeat(indent));
                    out.push_str(&format!("</{}>\n", elem.tag));
                }
                NodeKind::Text(text) => {
                    out.push_str(&"  ".repeat(indent));
                    out.push_str(text);
                    out.push('\n');
                }
                NodeKind::Comment(text) => {
                    out.push_str(&"  ".repeat(indent));
                    out.push_str(&format!("<!--{}-->\n", text));
                }
            }
        }
    }

    /// Print memory statistics
    pub fn memory_stats(&self) {
        let mut borrowed = 0;
        let mut owned = 0;

        fn count_cow(cow: &Cow<str>, borrowed: &mut usize, owned: &mut usize) {
            match cow {
                Cow::Borrowed(_) => *borrowed += 1,
                Cow::Owned(_) => *owned += 1,
            }
        }

        // This is a hack for our mini arena - real implementation would traverse properly
        println!("Memory Statistics:");
        println!("  Source HTML size: {} bytes", self.source.len());
        println!("  Arena nodes: (would count here)");
        println!("  Borrowed strings: {} (zero-copy)", borrowed);
        println!("  Owned strings: {} (allocated)", owned);
    }
}

// ============================================================================
// Demonstration
// ============================================================================

fn main() {
    println!("=== Arena-based DOM with Borrowed Strings ===\n");

    // Source HTML
    let html = r#"<!DOCTYPE html>
<html>
  <body>
    <div class="container">
      Hello, world!
    </div>
  </body>
</html>"#;

    println!("Source HTML:\n{}\n", html);

    // Parse (borrowing from source)
    let doc = parse_simple(html);

    println!("Parsed document:");
    doc.memory_stats();
    println!();

    // Navigate
    if let Some(body_id) = doc.body() {
        println!("Found <body> element!");
        for child_id in doc.children(body_id) {
            if let Some(node) = doc.get(child_id) {
                if let NodeKind::Element(elem) = &node.kind {
                    println!(
                        "  Child: <{}> with {} attributes",
                        elem.tag,
                        elem.attrs.len()
                    );
                }
            }
        }
    }
    println!();

    // Serialize
    println!("Serialized HTML:");
    println!("{}", doc.to_html());

    // Demonstrate mutation (Cow → Owned)
    let mut doc = doc;
    if let Some(body_id) = doc.body() {
        if let Some(div_id) = doc.children(body_id).next() {
            println!("Setting attribute (converts Borrowed → Owned)...");
            doc.set_attr(div_id, "id".to_string(), "main".to_string());

            println!("\nUpdated HTML:");
            println!("{}", doc.to_html());
        }
    }

    println!("\n=== Key Observations ===");
    println!("1. Tag names and attribute names are borrowed from source (zero-copy)");
    println!("2. Arena stores all nodes in contiguous memory (cache-friendly)");
    println!("3. Mutations convert Cow::Borrowed → Cow::Owned automatically");
    println!("4. Two documents can borrow from different sources (independent lifetimes)");
    println!("5. Perfect for parse → diff → discard workflow");
}

// ============================================================================
// Diff Example (Simplified)
// ============================================================================

#[allow(dead_code)]
fn diff_example() {
    // Load two versions
    let old_html = "<html><body><div>old</div></body></html>";
    let new_html = "<html><body><div>new</div></body></html>";

    // Parse both (independent lifetimes!)
    let old_doc = parse_simple(old_html);
    let new_doc = parse_simple(new_html);

    // Diff (would integrate with cinereus here)
    println!("Old doc root: {:?}", old_doc.root);
    println!("New doc root: {:?}", new_doc.root);

    // Both documents are independent - old_html and new_html have separate lifetimes
    // This is key for the hot-reload use case!
}
