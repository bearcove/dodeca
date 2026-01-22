//! HTML5 parser using html5ever's TreeSink.
//!
//! This implements TreeSink to build hotmeal DOM types using html5ever's tree
//! construction algorithm, which includes browser-compatible error recovery.

use crate::diff::{Content, Element as DiffElement};
use crate::untyped_dom::{Document, Element, Namespace, Node};
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, QualName, namespace_url, ns};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use tendril::StrTendril;

/// Parse an HTML string into an untyped Element tree (body content only).
///
/// This is useful for diffing and patching. Returns the body element with all its children.
pub fn parse_untyped(html: &str) -> DiffElement {
    let sink = HtmlSink::default();
    let sink = html5ever::parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_diff_element()
}

/// Parse an HTML string into an untyped Document.
///
/// This is the primary parsing function - it accepts any HTML that browsers accept,
/// using html5ever's full error recovery, and produces a simple Element/Node tree.
///
/// # Example
///
/// ```rust
/// use hotmeal::parse_document;
/// use hotmeal::untyped_dom::Node;
///
/// let doc = parse_document("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
/// assert_eq!(doc.doctype, Some("html".to_string()));
///
/// if let Some(body) = doc.body() {
///     if let Some(Node::Element(p)) = body.children.first() {
///         assert_eq!(p.tag, "p");
///     }
/// }
/// ```
pub fn parse_document(html: &str) -> Document {
    let sink = HtmlSink::default();
    let sink = html5ever::parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_document()
}

/// Parse an HTML fragment and return the body element.
///
/// Use this when you have an HTML fragment (not a complete document).
/// html5ever will wrap it appropriately.
///
/// # Example
///
/// ```rust
/// use hotmeal::parse_body;
/// use hotmeal::untyped_dom::Node;
///
/// let body = parse_body("<p>Hello!</p><p>World!</p>");
/// assert_eq!(body.tag, "body");
/// assert_eq!(body.children.len(), 2);
/// ```
pub fn parse_body(html: &str) -> Element {
    let sink = HtmlSink::default();
    let sink = html5ever::parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_body_element()
}

/// A node handle for our TreeSink - can be document, element, text, or comment
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeHandle(usize);

/// Internal node representation during parsing
#[derive(Clone, Debug)]
enum ParseNode {
    Document {
        children: Vec<NodeHandle>,
    },
    Element {
        name: QualName,
        attrs: Vec<(String, String)>,
        children: Vec<NodeHandle>,
    },
    Text(String),
    Comment(String),
}

/// TreeSink that builds hotmeal DOM types
#[derive(Default)]
struct HtmlSink {
    next_id: Cell<usize>,
    nodes: RefCell<HashMap<NodeHandle, ParseNode>>,
    document_handle: Cell<Option<NodeHandle>>,
    doctype: RefCell<Option<String>>,
}

impl HtmlSink {
    fn alloc(&self, node: ParseNode) -> NodeHandle {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let handle = NodeHandle(id);
        self.nodes.borrow_mut().insert(handle, node);
        handle
    }

    fn append_child_to(&self, parent: NodeHandle, child: NodeHandle) {
        let mut nodes = self.nodes.borrow_mut();
        match nodes.get_mut(&parent) {
            Some(ParseNode::Element { children, .. }) => children.push(child),
            Some(ParseNode::Document { children }) => children.push(child),
            _ => {}
        }
    }

    /// Convert the parsed tree to an untyped Element tree for diff (body only).
    fn into_diff_element(self) -> DiffElement {
        let nodes = self.nodes.into_inner();
        let doc_handle = self.document_handle.get().unwrap();

        // Find html > body element
        if let Some(ParseNode::Document { children }) = nodes.get(&doc_handle) {
            for &child in children {
                if let Some(ParseNode::Element { name, children, .. }) = nodes.get(&child)
                    && name.local.as_ref() == "html"
                {
                    for &html_child in children {
                        if let Some(ParseNode::Element { name, .. }) = nodes.get(&html_child)
                            && name.local.as_ref() == "body"
                        {
                            return Self::build_diff_element(&nodes, html_child);
                        }
                    }
                }
            }
        }

        // Fallback: empty body
        DiffElement {
            tag: "body".to_string(),
            attrs: HashMap::new(),
            children: vec![],
        }
    }

    /// Recursively build a diff Element from a ParseNode.
    fn build_diff_element(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> DiffElement {
        if let Some(ParseNode::Element {
            name,
            attrs,
            children,
        }) = nodes.get(&handle)
        {
            DiffElement {
                tag: name.local.to_string(),
                attrs: attrs.iter().cloned().collect(),
                children: children
                    .iter()
                    .filter_map(|&child| Self::build_diff_content(nodes, child))
                    .collect(),
            }
        } else {
            DiffElement {
                tag: "".to_string(),
                attrs: HashMap::new(),
                children: vec![],
            }
        }
    }

    /// Build diff Content from a ParseNode.
    fn build_diff_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<Content> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(Content::Text(t.clone())),
            ParseNode::Element { .. } => {
                Some(Content::Element(Self::build_diff_element(nodes, handle)))
            }
            ParseNode::Document { .. } | ParseNode::Comment(_) => None,
        }
    }

    /// Convert the parsed tree to a Document (new untyped DOM).
    fn into_document(self) -> Document {
        let nodes = self.nodes.into_inner();
        let doc_handle = self.document_handle.get().unwrap();
        let doctype = self.doctype.into_inner();

        // Find html element
        if let Some(ParseNode::Document { children }) = nodes.get(&doc_handle) {
            for &child in children {
                if let Some(ParseNode::Element { name, .. }) = nodes.get(&child)
                    && name.local.as_ref() == "html"
                {
                    let root = Self::build_element(&nodes, child);
                    return Document { doctype, root };
                }
            }
        }

        // Fallback: create minimal document
        Document {
            doctype,
            root: Element::new("html"),
        }
    }

    /// Convert the parsed tree to just the body Element.
    fn into_body_element(self) -> Element {
        let nodes = self.nodes.into_inner();
        let doc_handle = self.document_handle.get().unwrap();

        // Find html > body element
        if let Some(ParseNode::Document { children }) = nodes.get(&doc_handle) {
            for &child in children {
                if let Some(ParseNode::Element { name, children, .. }) = nodes.get(&child)
                    && name.local.as_ref() == "html"
                {
                    for &html_child in children {
                        if let Some(ParseNode::Element { name, .. }) = nodes.get(&html_child)
                            && name.local.as_ref() == "body"
                        {
                            return Self::build_element(&nodes, html_child);
                        }
                    }
                }
            }
        }

        // Fallback: empty body
        Element::new("body")
    }

    /// Recursively build an Element from a ParseNode.
    fn build_element(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Element {
        if let Some(ParseNode::Element {
            name,
            attrs,
            children,
        }) = nodes.get(&handle)
        {
            let ns = Self::map_namespace(&name.ns);
            let tag = name.local.to_string();

            Element {
                tag,
                ns,
                attrs: attrs.iter().cloned().collect(),
                children: children
                    .iter()
                    .filter_map(|&child| Self::build_node(nodes, child))
                    .collect(),
            }
        } else {
            Element::default()
        }
    }

    /// Build a Node from a ParseNode.
    fn build_node(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Option<Node> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(Node::Text(t.clone())),
            ParseNode::Comment(t) => Some(Node::Comment(t.clone())),
            ParseNode::Element { .. } => Some(Node::Element(Self::build_element(nodes, handle))),
            ParseNode::Document { .. } => None,
        }
    }

    /// Map html5ever namespace to our Namespace enum.
    fn map_namespace(ns: &html5ever::Namespace) -> Namespace {
        if *ns == ns!(html) {
            Namespace::Html
        } else if *ns == ns!(svg) {
            Namespace::Svg
        } else if *ns == ns!(mathml) {
            Namespace::MathMl
        } else {
            Namespace::Html
        }
    }
}

impl TreeSink for HtmlSink {
    type Handle = NodeHandle;
    type Output = Self;
    type ElemName<'a> = &'a QualName;

    fn finish(self) -> Self::Output {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {
        // Ignore parse errors - we want to accept everything
    }

    fn get_document(&self) -> Self::Handle {
        if let Some(h) = self.document_handle.get() {
            h
        } else {
            let h = self.alloc(ParseNode::Document {
                children: Vec::new(),
            });
            self.document_handle.set(Some(h));
            h
        }
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        let nodes = self.nodes.borrow();
        if let Some(ParseNode::Element { name, .. }) = nodes.get(target) {
            // SAFETY: The name is stored in the HashMap and won't be moved
            // while we hold a reference to it through the borrow.
            unsafe { &*(name as *const QualName) }
        } else {
            panic!("elem_name called on non-element")
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let attrs = attrs
            .into_iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect();
        self.alloc(ParseNode::Element {
            name,
            attrs,
            children: Vec::new(),
        })
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        self.alloc(ParseNode::Comment(text.to_string()))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Self::Handle {
        self.alloc(ParseNode::Text(String::new()))
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.append_child_to(*parent, node);
            }
            NodeOrText::AppendText(text) => {
                let mut nodes = self.nodes.borrow_mut();
                // Merge adjacent text nodes
                let last_child_id = match nodes.get(parent) {
                    Some(ParseNode::Element { children, .. }) => children.last().copied(),
                    Some(ParseNode::Document { children }) => children.last().copied(),
                    _ => None,
                };

                if let Some(last_id) = last_child_id
                    && let Some(ParseNode::Text(existing)) = nodes.get_mut(&last_id)
                {
                    existing.push_str(&text);
                    return;
                }
                drop(nodes);
                let text_id = self.alloc(ParseNode::Text(text.to_string()));
                self.append_child_to(*parent, text_id);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        _element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.append(prev_element, child);
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        *self.doctype.borrow_mut() = Some(name.to_string());
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        *target
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let new_id = match new_node {
            NodeOrText::AppendNode(n) => n,
            NodeOrText::AppendText(text) => self.alloc(ParseNode::Text(text.to_string())),
        };

        let mut nodes = self.nodes.borrow_mut();
        for node in nodes.values_mut() {
            match node {
                ParseNode::Element { children, .. } | ParseNode::Document { children } => {
                    if let Some(pos) = children.iter().position(|&c| c == *sibling) {
                        children.insert(pos, new_id);
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(ParseNode::Element {
            attrs: existing, ..
        }) = nodes.get_mut(target)
        {
            for attr in attrs {
                let name = attr.name.local.to_string();
                if !existing.iter().any(|(k, _)| k == &name) {
                    existing.push((name, attr.value.to_string()));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        let mut nodes = self.nodes.borrow_mut();
        for node in nodes.values_mut() {
            match node {
                ParseNode::Element { children, .. } | ParseNode::Document { children } => {
                    children.retain(|&c| c != *target);
                }
                _ => {}
            }
        }
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut nodes = self.nodes.borrow_mut();
        let children = match nodes.get_mut(node) {
            Some(ParseNode::Element { children, .. }) => std::mem::take(children),
            Some(ParseNode::Document { children }) => std::mem::take(children),
            _ => return,
        };
        match nodes.get_mut(new_parent) {
            Some(ParseNode::Element {
                children: new_children,
                ..
            }) => new_children.extend(children),
            Some(ParseNode::Document {
                children: new_children,
            }) => new_children.extend(children),
            _ => {}
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_document() {
        let doc = parse_document("<!DOCTYPE html><html><body><p>Hello!</p></body></html>");
        assert_eq!(doc.doctype, Some("html".to_string()));
        assert_eq!(doc.root.tag, "html");
        assert!(doc.body().is_some());
    }

    #[test]
    fn test_parse_body() {
        let body = parse_body("<p>Hello!</p><p>World!</p>");
        assert_eq!(body.tag, "body");
        assert_eq!(body.children.len(), 2);
    }

    #[test]
    fn test_parse_body_with_attrs() {
        let body = parse_body("<div id=\"main\" class=\"container\"><p>Test</p></div>");
        if let Some(Node::Element(div)) = body.children.first() {
            assert_eq!(div.tag, "div");
            assert_eq!(div.attrs.get("id"), Some(&"main".to_string()));
            assert_eq!(div.attrs.get("class"), Some(&"container".to_string()));
        } else {
            panic!("expected element");
        }
    }

    #[test]
    fn test_parse_with_comments() {
        // Note: html5ever puts comments that appear before body content
        // outside the body element. Put the comment inside.
        let body = parse_body("<p>Text</p><!-- comment -->");
        assert_eq!(body.children.len(), 2);
        assert!(matches!(body.children[0], Node::Element(_)));
        assert!(matches!(body.children[1], Node::Comment(_)));
    }

    #[test]
    fn test_error_recovery() {
        // Browser auto-closes p tags
        let body = parse_body("<p>outer<p>inner</p>");
        // Should have 2 paragraphs due to auto-closing
        let p_count = body
            .children
            .iter()
            .filter(|n| matches!(n, Node::Element(e) if e.tag == "p"))
            .count();
        assert!(p_count >= 2, "expected at least 2 p elements");
    }
}
