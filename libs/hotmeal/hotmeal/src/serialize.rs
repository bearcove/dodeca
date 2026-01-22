//! HTML5-correct serializer for untyped DOM.
//!
//! This module provides serialization of `Document` and `Element` types
//! to HTML strings, following HTML5 serialization rules:
//!
//! - Void elements never get end tags
//! - Text content is properly escaped
//! - Attribute values are escaped and double-quoted
//! - Raw text elements (script, style) are not escaped
//! - RCDATA elements (title, textarea) escape only `&` and `<`
//! - Foreign content (SVG/MathML) can use self-closing syntax

use crate::untyped_dom::{Document, Element, Namespace, Node};
use std::fmt::Write;

/// Options for HTML serialization.
#[derive(Clone, Debug)]
pub struct SerializeOptions {
    /// Whether to pretty-print with indentation (default: false for minified output)
    pub pretty: bool,
    /// Indentation string for pretty-printing (default: "  ")
    pub indent: String,
    /// Whether to sort attributes alphabetically (default: false).
    /// When false, attribute order is nondeterministic (HashMap iteration order).
    /// Enable this for deterministic output (e.g., for snapshots, caching, reproducible diffs).
    pub sort_attributes: bool,
    /// Whether to escape `</script` sequences in script content (default: true for safety)
    pub escape_script_end_tags: bool,
}

impl Default for SerializeOptions {
    fn default() -> Self {
        Self {
            pretty: false,
            indent: "  ".to_string(),
            sort_attributes: false,
            escape_script_end_tags: true,
        }
    }
}

impl SerializeOptions {
    /// Create new default options (minified output).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable pretty-printing with default indentation.
    pub fn pretty(mut self) -> Self {
        self.pretty = true;
        self
    }

    /// Set a custom indentation string (implies pretty-printing).
    pub fn with_indent(mut self, indent: impl Into<String>) -> Self {
        self.indent = indent.into();
        self.pretty = true;
        self
    }

    /// Enable sorting attributes alphabetically for deterministic output.
    pub fn sort_attributes(mut self) -> Self {
        self.sort_attributes = true;
        self
    }

    /// Disable escaping `</script` in script content (not recommended).
    pub fn no_escape_script_end_tags(mut self) -> Self {
        self.escape_script_end_tags = false;
        self
    }
}

/// Serialize a document to an HTML string.
pub fn serialize_document(doc: &Document, opts: &SerializeOptions) -> String {
    let mut out = String::new();
    let mut ser = Serializer::new(&mut out, opts);
    ser.write_document(doc);
    out
}

/// Serialize an element and its children to an HTML string.
pub fn serialize_element(elem: &Element, opts: &SerializeOptions) -> String {
    let mut out = String::new();
    let mut ser = Serializer::new(&mut out, opts);
    ser.write_element(elem);
    out
}

/// Serialize a slice of nodes to an HTML string (fragment).
pub fn serialize_fragment(nodes: &[Node], opts: &SerializeOptions) -> String {
    let mut out = String::new();
    let mut ser = Serializer::new(&mut out, opts);
    for node in nodes {
        ser.write_node(node);
    }
    out
}

/// HTML5 void elements - these never have end tags.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Raw text elements - content is not escaped.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style"];

/// RCDATA elements - only `&` and `<` are escaped.
const RCDATA_ELEMENTS: &[&str] = &["title", "textarea"];

/// Check if a tag is a void element.
fn is_void_element(tag: &str) -> bool {
    VOID_ELEMENTS.contains(&tag.to_ascii_lowercase().as_str())
}

/// Check if a tag is a raw text element (script, style).
fn is_raw_text_element(tag: &str) -> bool {
    RAW_TEXT_ELEMENTS.contains(&tag.to_ascii_lowercase().as_str())
}

/// Check if a tag is an RCDATA element (title, textarea).
fn is_rcdata_element(tag: &str) -> bool {
    RCDATA_ELEMENTS.contains(&tag.to_ascii_lowercase().as_str())
}

struct Serializer<'a, W: Write> {
    out: &'a mut W,
    options: &'a SerializeOptions,
    depth: usize,
}

impl<'a, W: Write> Serializer<'a, W> {
    fn new(out: &'a mut W, options: &'a SerializeOptions) -> Self {
        Self {
            out,
            options,
            depth: 0,
        }
    }

    fn write_indent(&mut self) {
        if self.options.pretty {
            for _ in 0..self.depth {
                let _ = write!(self.out, "{}", self.options.indent);
            }
        }
    }

    fn write_newline(&mut self) {
        if self.options.pretty {
            let _ = writeln!(self.out);
        }
    }

    /// Escape text content for normal HTML elements.
    fn write_text_escaped(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '&' => {
                    let _ = write!(self.out, "&amp;");
                }
                '<' => {
                    let _ = write!(self.out, "&lt;");
                }
                '>' => {
                    let _ = write!(self.out, "&gt;");
                }
                _ => {
                    let _ = write!(self.out, "{}", c);
                }
            }
        }
    }

    /// Escape text content for RCDATA elements (only & and <).
    fn write_rcdata_escaped(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '&' => {
                    let _ = write!(self.out, "&amp;");
                }
                '<' => {
                    let _ = write!(self.out, "&lt;");
                }
                _ => {
                    let _ = write!(self.out, "{}", c);
                }
            }
        }
    }

    /// Write raw text content, optionally escaping script end tags.
    fn write_raw_text(&mut self, text: &str, tag: &str) {
        if self.options.escape_script_end_tags && tag.eq_ignore_ascii_case("script") {
            // Escape </script to prevent premature closing
            // Use ASCII case-insensitive matching on original bytes to avoid
            // index misalignment from Unicode lowercasing
            const PATTERN: &[u8] = b"</script";
            let bytes = text.as_bytes();
            let mut last_end = 0;

            for i in 0..bytes.len().saturating_sub(PATTERN.len() - 1) {
                if bytes[i..].len() >= PATTERN.len()
                    && bytes[i..i + PATTERN.len()].eq_ignore_ascii_case(PATTERN)
                {
                    let _ = write!(self.out, "{}", &text[last_end..i]);
                    let _ = write!(self.out, "<\\/script");
                    last_end = i + PATTERN.len();
                }
            }
            let _ = write!(self.out, "{}", &text[last_end..]);
        } else {
            let _ = write!(self.out, "{}", text);
        }
    }

    /// Escape attribute value and write it double-quoted.
    fn write_attr_value_escaped(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '&' => {
                    let _ = write!(self.out, "&amp;");
                }
                '<' => {
                    let _ = write!(self.out, "&lt;");
                }
                '>' => {
                    let _ = write!(self.out, "&gt;");
                }
                '"' => {
                    let _ = write!(self.out, "&quot;");
                }
                _ => {
                    let _ = write!(self.out, "{}", c);
                }
            }
        }
    }

    fn write_attr(&mut self, name: &str, value: &str) {
        let _ = write!(self.out, " {}=\"", name);
        self.write_attr_value_escaped(value);
        let _ = write!(self.out, "\"");
    }

    fn write_document(&mut self, doc: &Document) {
        // DOCTYPE
        if let Some(doctype) = &doc.doctype {
            let _ = write!(self.out, "<!DOCTYPE {}>", doctype);
            self.write_newline();
        }

        self.write_element(&doc.root);
    }

    fn write_element(&mut self, elem: &Element) {
        let tag = &elem.tag;
        let is_void = is_void_element(tag);
        let is_raw = is_raw_text_element(tag);
        let is_rcdata = is_rcdata_element(tag);
        let is_foreign = elem.ns != Namespace::Html;

        // Opening tag
        self.write_indent();
        let _ = write!(self.out, "<{}", tag);

        // Attributes
        if self.options.sort_attributes {
            let mut attrs: Vec<_> = elem.attrs.iter().collect();
            attrs.sort_by_key(|(k, _)| *k);
            for (name, value) in attrs {
                self.write_attr(name, value);
            }
        } else {
            for (name, value) in &elem.attrs {
                self.write_attr(name, value);
            }
        }

        // Handle void elements
        if is_void {
            let _ = write!(self.out, ">");
            self.write_newline();
            return;
        }

        // Handle foreign content with self-closing syntax if no children
        if is_foreign && elem.children.is_empty() {
            let _ = write!(self.out, "/>");
            self.write_newline();
            return;
        }

        let _ = write!(self.out, ">");

        // Handle children
        if elem.children.is_empty() {
            // Empty element
            let _ = write!(self.out, "</{}>", tag);
            self.write_newline();
            return;
        }

        // Check if children are all inline (text only)
        let all_text = elem.children.iter().all(|c| matches!(c, Node::Text(_)));

        if is_raw || is_rcdata {
            // Raw text or RCDATA elements: write children inline
            for child in &elem.children {
                if let Node::Text(text) = child {
                    if is_raw {
                        self.write_raw_text(text, tag);
                    } else {
                        self.write_rcdata_escaped(text);
                    }
                }
            }
            let _ = write!(self.out, "</{}>", tag);
            self.write_newline();
        } else if all_text && !self.options.pretty {
            // Inline text content (minified)
            for child in &elem.children {
                if let Node::Text(text) = child {
                    self.write_text_escaped(text);
                }
            }
            let _ = write!(self.out, "</{}>", tag);
        } else if all_text {
            // Inline text content (pretty)
            for child in &elem.children {
                if let Node::Text(text) = child {
                    self.write_text_escaped(text);
                }
            }
            let _ = write!(self.out, "</{}>", tag);
            self.write_newline();
        } else {
            // Block content with children
            self.write_newline();
            self.depth += 1;
            for child in &elem.children {
                self.write_node(child);
            }
            self.depth -= 1;
            self.write_indent();
            let _ = write!(self.out, "</{}>", tag);
            self.write_newline();
        }
    }

    fn write_node(&mut self, node: &Node) {
        match node {
            Node::Element(elem) => {
                self.write_element(elem);
            }
            Node::Text(text) => {
                // Preserve all text nodes including whitespace-only ones
                // to avoid lossy serialization (whitespace can be significant
                // between inline elements, in <pre>, etc.)
                self.write_indent();
                self.write_text_escaped(text);
                if self.options.pretty && !text.is_empty() {
                    self.write_newline();
                }
            }
            Node::Comment(text) => {
                self.write_indent();
                // HTML comments: escape -- to prevent early closing
                let safe_text = text.replace("--", "- -");
                let _ = write!(self.out, "<!--{}-->", safe_text);
                self.write_newline();
            }
        }
    }
}

// =============================================================================
// Convenience methods on Element
// =============================================================================

impl Element {
    /// Serialize this element to an HTML string with default options.
    pub fn to_html(&self) -> String {
        serialize_element(self, &SerializeOptions::default())
    }

    /// Serialize this element to a pretty-printed HTML string.
    pub fn to_html_pretty(&self) -> String {
        serialize_element(self, &SerializeOptions::default().pretty())
    }

    /// Serialize this element with custom options.
    pub fn to_html_with_options(&self, opts: &SerializeOptions) -> String {
        serialize_element(self, opts)
    }
}

impl Document {
    /// Serialize this document to an HTML string with default options.
    pub fn to_html(&self) -> String {
        serialize_document(self, &SerializeOptions::default())
    }

    /// Serialize this document to a pretty-printed HTML string.
    pub fn to_html_pretty(&self) -> String {
        serialize_document(self, &SerializeOptions::default().pretty())
    }

    /// Serialize this document with custom options.
    pub fn to_html_with_options(&self, opts: &SerializeOptions) -> String {
        serialize_document(self, opts)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_void_elements() {
        let mut div = Element::new("div");
        div.push_child(Node::Element(Element::new("br")));
        div.push_child(Node::Element(Element::new("input")));

        let html = div.to_html();
        assert!(html.contains("<br>"));
        assert!(!html.contains("</br>"));
        assert!(html.contains("<input>"));
        assert!(!html.contains("</input>"));
    }

    #[test]
    fn test_text_escaping() {
        let mut p = Element::new("p");
        p.push_text("<script>alert('xss')</script>");

        let html = p.to_html();
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn test_attribute_escaping() {
        let mut a = Element::new("a");
        a.set_attr("href", "test?a=1&b=2");
        a.set_attr("title", "Say \"hello\"");

        let html = a.to_html();
        assert!(html.contains("href=\"test?a=1&amp;b=2\""));
        assert!(html.contains("title=\"Say &quot;hello&quot;\""));
    }

    #[test]
    fn test_raw_text_elements() {
        let mut script = Element::new("script");
        script.push_text("if (a < b && c > d) {}");

        let opts = SerializeOptions::default().no_escape_script_end_tags();
        let html = script.to_html_with_options(&opts);
        // Raw text should NOT be escaped
        assert!(html.contains("a < b && c > d"));
    }

    #[test]
    fn test_script_end_tag_escaping() {
        let mut script = Element::new("script");
        script.push_text("var x = '</script>';");

        let opts = SerializeOptions::default();
        let html = script.to_html_with_options(&opts);
        // </script should be escaped to prevent XSS
        assert!(html.contains("<\\/script"));
    }

    #[test]
    fn test_rcdata_elements() {
        let mut title = Element::new("title");
        title.push_text("Test & <Demo>");

        let html = title.to_html();
        // RCDATA: & and < are escaped, > is not
        assert!(html.contains("Test &amp; &lt;Demo>"));
    }

    #[test]
    fn test_foreign_content_self_closing() {
        let rect = Element::with_namespace("rect", Namespace::Svg);

        let html = rect.to_html();
        // Foreign content can use self-closing syntax
        assert!(html.contains("<rect/>") || html.contains("<rect />"));
    }

    #[test]
    fn test_document_serialization() {
        let doc = Document::html5();
        let html = doc.to_html();

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<html>"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn test_comment_serialization() {
        let mut div = Element::new("div");
        div.push_child(Node::Comment("This is a comment".to_string()));

        let html = div.to_html();
        assert!(html.contains("<!--This is a comment-->"));
    }

    #[test]
    fn test_comment_with_dashes() {
        let mut div = Element::new("div");
        div.push_child(Node::Comment("Test -- comment".to_string()));

        let html = div.to_html();
        // -- should be escaped to prevent early closing
        assert!(html.contains("<!--Test - - comment-->"));
    }

    #[test]
    fn test_pretty_print() {
        let mut div = Element::new("div");
        let mut p = Element::new("p");
        p.push_text("Hello");
        div.push_element(p);

        let html = div.to_html_pretty();
        assert!(html.contains('\n'));
        assert!(html.contains("  ")); // default indent
    }

    #[test]
    fn test_sorted_attributes() {
        let mut elem = Element::new("div");
        elem.set_attr("zebra", "1");
        elem.set_attr("alpha", "2");
        elem.set_attr("mike", "3");

        let opts = SerializeOptions::default().sort_attributes();
        let html = elem.to_html_with_options(&opts);

        let alpha_pos = html.find("alpha").unwrap();
        let mike_pos = html.find("mike").unwrap();
        let zebra_pos = html.find("zebra").unwrap();

        assert!(alpha_pos < mike_pos);
        assert!(mike_pos < zebra_pos);
    }
}
