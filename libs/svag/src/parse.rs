//! SVG parsing from XML.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use crate::ast::*;
use crate::error::SavageError;

/// Parse an SVG string into a Document.
pub fn parse_svg(svg: &str) -> Result<Document, SavageError> {
    let mut reader = Reader::from_str(svg);

    let mut xml_declaration = None;
    let mut doctype = None;
    let mut root = None;

    loop {
        match reader.read_event()? {
            Event::Decl(decl) => {
                xml_declaration = Some(XmlDeclaration {
                    version: String::from_utf8_lossy(decl.version()?.as_ref()).into_owned(),
                    encoding: decl
                        .encoding()
                        .transpose()
                        .ok()
                        .flatten()
                        .map(|e| String::from_utf8_lossy(e.as_ref()).into_owned()),
                    standalone: decl.standalone().transpose().ok().flatten().map(|s| {
                        let s = String::from_utf8_lossy(s.as_ref());
                        s == "yes"
                    }),
                });
            }
            Event::DocType(dt) => {
                doctype = Some(String::from_utf8_lossy(&dt).into_owned());
            }
            Event::Start(start) => {
                root = Some(parse_element(&mut reader, &start)?);
                break;
            }
            Event::Empty(start) => {
                root = Some(parse_empty_element(&start)?);
                break;
            }
            Event::Comment(_) | Event::Text(_) | Event::PI(_) => {
                // Skip top-level comments/whitespace/PIs before root
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let root = root.ok_or_else(|| SavageError::InvalidSvg("No root element found".into()))?;

    Ok(Document {
        xml_declaration,
        doctype,
        root,
    })
}

fn parse_element(reader: &mut Reader<&[u8]>, start: &BytesStart) -> Result<Element, SavageError> {
    let mut element = parse_element_start(start)?;

    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                element
                    .children
                    .push(Node::Element(parse_element(reader, &start)?));
            }
            Event::Empty(start) => {
                element
                    .children
                    .push(Node::Element(parse_empty_element(&start)?));
            }
            Event::End(_) => {
                break;
            }
            Event::Text(text) => {
                let text = text.unescape()?;
                if !text.trim().is_empty() || !element.children.is_empty() {
                    element.children.push(Node::Text(text.into_owned()));
                }
            }
            Event::Comment(comment) => {
                element
                    .children
                    .push(Node::Comment(String::from_utf8_lossy(&comment).into_owned()));
            }
            Event::CData(cdata) => {
                element
                    .children
                    .push(Node::CData(String::from_utf8_lossy(&cdata).into_owned()));
            }
            Event::PI(pi) => {
                let content = String::from_utf8_lossy(&pi).into_owned();
                let (target, rest) = content
                    .split_once(char::is_whitespace)
                    .map(|(t, r)| (t.to_string(), Some(r.to_string())))
                    .unwrap_or_else(|| (content, None));
                element
                    .children
                    .push(Node::ProcessingInstruction { target, content: rest });
            }
            Event::Eof => {
                return Err(SavageError::InvalidSvg("Unexpected end of file".into()));
            }
            _ => {}
        }
    }

    Ok(element)
}

fn parse_empty_element(start: &BytesStart) -> Result<Element, SavageError> {
    parse_element_start(start)
}

fn parse_element_start(start: &BytesStart) -> Result<Element, SavageError> {
    let name_bytes = start.name();
    let name = std::str::from_utf8(name_bytes.as_ref())?;

    let mut element = Element {
        name: QName::parse(name),
        attributes: Vec::new(),
        children: Vec::new(),
    };

    for attr in start.attributes() {
        let attr = attr.map_err(|e| SavageError::InvalidSvg(format!("Invalid attribute: {}", e)))?;
        let key = std::str::from_utf8(attr.key.as_ref())?;
        let value = attr.unescape_value()?;
        element.attributes.push(Attribute {
            name: QName::parse(key),
            value: value.into_owned(),
        });
    }

    Ok(element)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_svg() {
        let svg = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
    <rect x="10" y="10" width="80" height="80" fill="red"/>
</svg>"#;

        let doc = parse_svg(svg).unwrap();
        assert!(doc.xml_declaration.is_some());
        assert!(doc.root.is("svg"));
        assert_eq!(doc.root.get_attr("width"), Some("100"));
    }

    #[test]
    fn test_parse_with_comments() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
    <!-- This is a comment -->
    <rect/>
</svg>"#;

        let doc = parse_svg(svg).unwrap();
        // whitespace text nodes + comment + rect
        let comments: Vec<_> = doc.root.children.iter().filter(|n| matches!(n, Node::Comment(_))).collect();
        assert_eq!(comments.len(), 1);
    }

    #[test]
    fn test_parse_namespaced() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
    <use xlink:href="#foo"/>
</svg>"##;

        let doc = parse_svg(svg).unwrap();
        let ns = doc.root.namespaces();
        assert!(ns.contains_key(&None)); // default namespace
        assert!(ns.contains_key(&Some("xlink")));
    }
}
