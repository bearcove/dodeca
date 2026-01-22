//! Untyped DOM types for HTML parsing and serialization.
//!
//! This module provides a simpler, untyped representation of HTML documents
//! that doesn't enforce content model rules. It's used for:
//! - Parsing any HTML that browsers accept
//! - Diffing and patching HTML documents
//! - Serializing HTML with proper escaping
//!
//! # Example
//!
//! ```rust,ignore
//! use hotmeal::untyped_dom::{Document, Element, Node, Namespace};
//!
//! let doc = hotmeal::parse_document("<html><body><p>Hello!</p></body></html>").unwrap();
//! println!("Doctype: {:?}", doc.doctype);
//! ```

use std::collections::HashMap;

/// XML/HTML namespace for elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, facet::Facet)]
#[repr(u8)]
pub enum Namespace {
    /// HTML namespace (default)
    #[default]
    Html,
    /// SVG namespace
    Svg,
    /// MathML namespace
    MathMl,
}

impl Namespace {
    /// Returns the namespace URI.
    pub fn uri(&self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::Svg => "http://www.w3.org/2000/svg",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }
}

/// An ordered collection of attributes with first-wins semantics.
///
/// When parsing HTML, if an attribute appears multiple times, only the first
/// occurrence is kept (matching browser behavior).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attributes {
    /// Ordered list of (name, value) pairs
    entries: Vec<(String, String)>,
}

impl Attributes {
    /// Create a new empty attribute collection.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Create from an iterator of (name, value) pairs.
    /// Enforces first-wins: if a name appears multiple times, only the first is kept.
    pub fn collect_from<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut attrs = Self::new();
        for (name, value) in iter {
            attrs.set_if_missing(name, value);
        }
        attrs
    }

    /// Get an attribute value by name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Set an attribute value. If the attribute already exists, updates its value.
    pub fn set(&mut self, name: String, value: String) {
        if let Some((_, v)) = self.entries.iter_mut().find(|(n, _)| n == &name) {
            *v = value;
        } else {
            self.entries.push((name, value));
        }
    }

    /// Set an attribute only if it doesn't already exist (first-wins semantics).
    pub fn set_if_missing(&mut self, name: String, value: String) {
        if !self.entries.iter().any(|(n, _)| n == &name) {
            self.entries.push((name, value));
        }
    }

    /// Remove an attribute by name. Returns the old value if it existed.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        if let Some(pos) = self.entries.iter().position(|(n, _)| n == name) {
            Some(self.entries.remove(pos).1)
        } else {
            None
        }
    }

    /// Check if an attribute exists.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|(n, _)| n == name)
    }

    /// Iterate over all attributes in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Get the number of attributes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if there are no attributes.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Convert to a HashMap (loses ordering but useful for compatibility).
    pub fn to_hashmap(&self) -> HashMap<String, String> {
        self.entries.iter().cloned().collect()
    }
}

/// DOM content - either an element, text, or comment.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum Node {
    /// An element node
    Element(Element),
    /// A text node
    #[facet(text)]
    Text(String),
    /// A comment node
    Comment(String),
}

impl Node {
    /// Returns true if this is an element node.
    pub fn is_element(&self) -> bool {
        matches!(self, Node::Element(_))
    }

    /// Returns true if this is a text node.
    pub fn is_text(&self) -> bool {
        matches!(self, Node::Text(_))
    }

    /// Returns true if this is a comment node.
    pub fn is_comment(&self) -> bool {
        matches!(self, Node::Comment(_))
    }

    /// Get as element reference.
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    /// Get as mutable element reference.
    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }

    /// Get as text reference.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Node::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Get text content of this node and all descendants.
    pub fn text_content(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::Comment(_) => String::new(),
            Node::Element(e) => e.text_content(),
        }
    }
}

/// An HTML/SVG/MathML element.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct Element {
    /// The tag name (lowercase for HTML, case-preserved for SVG/MathML)
    #[facet(tag)]
    pub tag: String,
    /// The namespace (Html, Svg, or MathMl)
    #[facet(skip)]
    pub ns: Namespace,
    /// Attributes as key-value pairs
    #[facet(flatten)]
    pub attrs: HashMap<String, String>,
    /// Child nodes
    #[facet(flatten)]
    pub children: Vec<Node>,
}

impl Element {
    /// Create a new element with the given tag name in the HTML namespace.
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            ns: Namespace::Html,
            attrs: HashMap::new(),
            children: Vec::new(),
        }
    }

    /// Create a new element with namespace.
    pub fn with_namespace(tag: impl Into<String>, ns: Namespace) -> Self {
        Self {
            tag: tag.into(),
            ns,
            attrs: HashMap::new(),
            children: Vec::new(),
        }
    }

    /// Get an attribute value.
    pub fn get_attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(|s| s.as_str())
    }

    /// Set an attribute value.
    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.attrs.insert(name.into(), value.into());
    }

    /// Remove an attribute.
    pub fn remove_attr(&mut self, name: &str) -> Option<String> {
        self.attrs.remove(name)
    }

    /// Add a child node.
    pub fn push_child(&mut self, child: Node) {
        self.children.push(child);
    }

    /// Add a text child.
    pub fn push_text(&mut self, text: impl Into<String>) {
        self.children.push(Node::Text(text.into()));
    }

    /// Add an element child.
    pub fn push_element(&mut self, element: Element) {
        self.children.push(Node::Element(element));
    }

    /// Get text content of this element and all descendants.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
                Node::Comment(_) => {}
            }
        }
    }

    /// Get mutable reference to children at a path.
    pub fn children_mut(&mut self, path: &[usize]) -> Result<&mut Vec<Node>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Node::Element(e) => e,
                Node::Text(_) => return Err("cannot navigate through text node".to_string()),
                Node::Comment(_) => return Err("cannot navigate through comment node".to_string()),
            };
        }
        Ok(&mut current.children)
    }

    /// Get mutable reference to attrs at a path.
    pub fn attrs_mut(&mut self, path: &[usize]) -> Result<&mut HashMap<String, String>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Node::Element(e) => e,
                Node::Text(_) => return Err("cannot navigate through text node".to_string()),
                Node::Comment(_) => return Err("cannot navigate through comment node".to_string()),
            };
        }
        Ok(&mut current.attrs)
    }

    /// Get mutable reference to a node at a path.
    pub fn get_node_mut(&mut self, path: &[usize]) -> Result<&mut Node, String> {
        if path.is_empty() {
            return Err("cannot get node at empty path".to_string());
        }
        let parent_path = &path[..path.len() - 1];
        let idx = path[path.len() - 1];
        let children = self.children_mut(parent_path)?;
        children
            .get_mut(idx)
            .ok_or_else(|| format!("index {idx} out of bounds"))
    }
}

impl Default for Element {
    fn default() -> Self {
        Self {
            tag: String::new(),
            ns: Namespace::Html,
            attrs: HashMap::new(),
            children: Vec::new(),
        }
    }
}

/// A complete HTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The DOCTYPE declaration (e.g., "html" for `<!DOCTYPE html>`)
    pub doctype: Option<String>,
    /// The root html element
    pub root: Element,
}

impl Document {
    /// Create a new document with the given root element.
    pub fn new(root: Element) -> Self {
        Self {
            doctype: None,
            root,
        }
    }

    /// Create a new HTML5 document with a body element.
    pub fn html5() -> Self {
        let mut html = Element::new("html");
        let head = Element::new("head");
        let body = Element::new("body");
        html.push_element(head);
        html.push_element(body);
        Self {
            doctype: Some("html".to_string()),
            root: html,
        }
    }

    /// Get the head element if present.
    pub fn head(&self) -> Option<&Element> {
        self.root.children.iter().find_map(|child| {
            if let Node::Element(e) = child
                && e.tag == "head"
            {
                return Some(e);
            }
            None
        })
    }

    /// Get the body element if present.
    pub fn body(&self) -> Option<&Element> {
        self.root.children.iter().find_map(|child| {
            if let Node::Element(e) = child
                && e.tag == "body"
            {
                return Some(e);
            }
            None
        })
    }

    /// Get mutable reference to the body element.
    pub fn body_mut(&mut self) -> Option<&mut Element> {
        self.root.children.iter_mut().find_map(|child| {
            if let Node::Element(e) = child
                && e.tag == "body"
            {
                return Some(e);
            }
            None
        })
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::html5()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attributes_first_wins() {
        let attrs = Attributes::collect_from([
            ("class".to_string(), "first".to_string()),
            ("class".to_string(), "second".to_string()),
            ("id".to_string(), "myid".to_string()),
        ]);

        assert_eq!(attrs.get("class"), Some("first"));
        assert_eq!(attrs.get("id"), Some("myid"));
        assert_eq!(attrs.len(), 2);
    }

    #[test]
    fn test_attributes_set_updates() {
        let mut attrs = Attributes::new();
        attrs.set("class".to_string(), "first".to_string());
        assert_eq!(attrs.get("class"), Some("first"));

        attrs.set("class".to_string(), "second".to_string());
        assert_eq!(attrs.get("class"), Some("second"));
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_attributes_remove() {
        let mut attrs = Attributes::collect_from([
            ("class".to_string(), "myclass".to_string()),
            ("id".to_string(), "myid".to_string()),
        ]);

        assert_eq!(attrs.remove("class"), Some("myclass".to_string()));
        assert_eq!(attrs.get("class"), None);
        assert_eq!(attrs.len(), 1);
    }

    #[test]
    fn test_element_text_content() {
        let mut div = Element::new("div");
        div.push_text("Hello ");
        let mut span = Element::new("span");
        span.push_text("world");
        div.push_element(span);
        div.push_text("!");

        assert_eq!(div.text_content(), "Hello world!");
    }

    #[test]
    fn test_document_structure() {
        let doc = Document::html5();
        assert_eq!(doc.doctype, Some("html".to_string()));
        assert_eq!(doc.root.tag, "html");
        assert!(doc.head().is_some());
        assert!(doc.body().is_some());
    }

    #[test]
    fn test_namespace() {
        let html_elem = Element::new("div");
        assert_eq!(html_elem.ns, Namespace::Html);

        let svg_elem = Element::with_namespace("rect", Namespace::Svg);
        assert_eq!(svg_elem.ns, Namespace::Svg);
    }
}
