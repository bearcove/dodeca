//! SVG serialization to minified XML.

use crate::ast::*;
use crate::Options;

/// Serialize a Document to a minified SVG string.
pub fn serialize(doc: &Document, options: &Options) -> String {
    let mut out = String::new();

    // XML declaration
    if !options.remove_xml_declaration {
        if let Some(ref decl) = doc.xml_declaration {
            out.push_str("<?xml version=\"");
            out.push_str(&decl.version);
            out.push('"');
            if let Some(ref enc) = decl.encoding {
                out.push_str(" encoding=\"");
                out.push_str(enc);
                out.push('"');
            }
            if let Some(standalone) = decl.standalone {
                out.push_str(" standalone=\"");
                out.push_str(if standalone { "yes" } else { "no" });
                out.push('"');
            }
            out.push_str("?>");
        }
    }

    // DOCTYPE
    if !options.remove_doctype {
        if let Some(ref dt) = doc.doctype {
            out.push_str("<!DOCTYPE ");
            out.push_str(dt);
            out.push('>');
        }
    }

    // Root element
    serialize_element(&mut out, &doc.root, options);

    out
}

fn serialize_element(out: &mut String, elem: &Element, options: &Options) {
    out.push('<');
    out.push_str(&elem.name.full_name());

    // Serialize attributes
    let mut attrs: Vec<_> = elem.attributes.iter().collect();
    if options.sort_attrs {
        attrs.sort_by(|a, b| {
            // xmlns declarations first, then by name
            let a_xmlns = a.name.is_xmlns();
            let b_xmlns = b.name.is_xmlns();
            match (a_xmlns, b_xmlns) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.full_name().cmp(&b.name.full_name()),
            }
        });
    }

    for attr in attrs {
        out.push(' ');
        out.push_str(&attr.name.full_name());
        out.push_str("=\"");
        push_escaped_attr(out, &attr.value);
        out.push('"');
    }

    // Children or self-closing
    if elem.children.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');

        for child in &elem.children {
            serialize_node(out, child, options);
        }

        out.push_str("</");
        out.push_str(&elem.name.full_name());
        out.push('>');
    }
}

fn serialize_node(out: &mut String, node: &Node, options: &Options) {
    match node {
        Node::Element(elem) => serialize_element(out, elem, options),
        Node::Text(text) => {
            // Minify whitespace in text nodes
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                push_escaped_text(out, trimmed);
            }
        }
        Node::Comment(comment) => {
            if !options.remove_comments {
                out.push_str("<!--");
                out.push_str(comment);
                out.push_str("-->");
            }
        }
        Node::CData(data) => {
            out.push_str("<![CDATA[");
            out.push_str(data);
            out.push_str("]]>");
        }
        Node::ProcessingInstruction { target, content } => {
            out.push_str("<?");
            out.push_str(target);
            if let Some(c) = content {
                out.push(' ');
                out.push_str(c);
            }
            out.push_str("?>");
        }
    }
}

fn push_escaped_attr(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn push_escaped_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_svg;

    #[test]
    fn test_serialize_simple() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#;
        let doc = parse_svg(svg).unwrap();
        let options = Options::default();
        let out = serialize(&doc, &options);
        assert_eq!(out, r#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#);
    }

    #[test]
    fn test_serialize_removes_xml_decl() {
        let svg = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        let doc = parse_svg(svg).unwrap();
        let options = Options::default();
        let out = serialize(&doc, &options);
        assert!(!out.starts_with("<?xml"));
    }

    #[test]
    fn test_serialize_removes_comments() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><!-- comment --><rect/></svg>"#;
        let doc = parse_svg(svg).unwrap();
        let options = Options::default();
        let out = serialize(&doc, &options);
        assert!(!out.contains("<!--"));
    }
}
