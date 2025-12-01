//! SVG Abstract Syntax Tree

use std::collections::HashMap;

/// A complete SVG document.
#[derive(Debug, Clone)]
pub struct Document {
    /// XML declaration (e.g., `<?xml version="1.0" encoding="UTF-8"?>`)
    pub xml_declaration: Option<XmlDeclaration>,
    /// DOCTYPE declaration
    pub doctype: Option<String>,
    /// The root SVG element
    pub root: Element,
}

/// XML declaration attributes.
#[derive(Debug, Clone)]
pub struct XmlDeclaration {
    pub version: String,
    pub encoding: Option<String>,
    pub standalone: Option<bool>,
}

/// An SVG/XML element.
#[derive(Debug, Clone)]
pub struct Element {
    /// Element name with optional prefix (e.g., "svg", "svg:rect", "xlink:href")
    pub name: QName,
    /// Attributes on this element
    pub attributes: Vec<Attribute>,
    /// Child nodes
    pub children: Vec<Node>,
}

/// A qualified name (possibly with namespace prefix).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QName {
    /// Namespace prefix (e.g., "svg", "xlink")
    pub prefix: Option<String>,
    /// Local name (e.g., "rect", "href")
    pub local: String,
}

impl QName {
    pub fn new(local: impl Into<String>) -> Self {
        Self {
            prefix: None,
            local: local.into(),
        }
    }

    pub fn with_prefix(prefix: impl Into<String>, local: impl Into<String>) -> Self {
        Self {
            prefix: Some(prefix.into()),
            local: local.into(),
        }
    }

    /// Parse a qualified name from a string like "prefix:local" or just "local".
    pub fn parse(s: &str) -> Self {
        if let Some((prefix, local)) = s.split_once(':') {
            Self::with_prefix(prefix, local)
        } else {
            Self::new(s)
        }
    }

    /// Check if this is a namespace declaration (xmlns or xmlns:prefix).
    pub fn is_xmlns(&self) -> bool {
        self.prefix.as_deref() == Some("xmlns") || (self.prefix.is_none() && self.local == "xmlns")
    }

    /// Get the full name as a string.
    pub fn full_name(&self) -> String {
        match &self.prefix {
            Some(p) => format!("{}:{}", p, self.local),
            None => self.local.clone(),
        }
    }
}

/// An attribute on an element.
#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: QName,
    pub value: String,
}

impl Attribute {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: QName::new(name),
            value: value.into(),
        }
    }
}

/// A node in the SVG tree.
#[derive(Debug, Clone)]
pub enum Node {
    /// An element node
    Element(Element),
    /// A text node
    Text(String),
    /// A comment node
    Comment(String),
    /// A CDATA section
    CData(String),
    /// A processing instruction (e.g., `<?xml-stylesheet ... ?>`)
    ProcessingInstruction { target: String, content: Option<String> },
}

impl Element {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: QName::new(name),
            attributes: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Get an attribute value by local name.
    pub fn get_attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name.local == name)
            .map(|a| a.value.as_str())
    }

    /// Set an attribute value.
    pub fn set_attr(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        if let Some(attr) = self.attributes.iter_mut().find(|a| a.name.local == name) {
            attr.value = value.into();
        } else {
            self.attributes.push(Attribute::new(name, value));
        }
    }

    /// Remove an attribute by local name.
    pub fn remove_attr(&mut self, name: &str) {
        self.attributes.retain(|a| a.name.local != name);
    }

    /// Check if this element has a specific local name.
    pub fn is(&self, name: &str) -> bool {
        self.name.local == name
    }

    /// Get all namespace declarations on this element.
    pub fn namespaces(&self) -> HashMap<Option<&str>, &str> {
        let mut ns = HashMap::new();
        for attr in &self.attributes {
            if attr.name.local == "xmlns" && attr.name.prefix.is_none() {
                ns.insert(None, attr.value.as_str());
            } else if attr.name.prefix.as_deref() == Some("xmlns") {
                ns.insert(Some(attr.name.local.as_str()), attr.value.as_str());
            }
        }
        ns
    }

    /// Iterate over child elements only (skip text, comments, etc.).
    pub fn child_elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Iterate over child elements mutably.
    pub fn child_elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }
}

impl Document {
    /// Recursively visit all elements in the document.
    pub fn for_each_element(&self, mut f: impl FnMut(&Element)) {
        fn visit(elem: &Element, f: &mut impl FnMut(&Element)) {
            f(elem);
            for child in elem.child_elements() {
                visit(child, f);
            }
        }
        visit(&self.root, &mut f);
    }

    /// Recursively visit all elements mutably.
    pub fn for_each_element_mut(&mut self, mut f: impl FnMut(&mut Element)) {
        fn visit(elem: &mut Element, f: &mut impl FnMut(&mut Element)) {
            f(elem);
            for child in elem.child_elements_mut() {
                visit(child, f);
            }
        }
        visit(&mut self.root, &mut f);
    }
}
