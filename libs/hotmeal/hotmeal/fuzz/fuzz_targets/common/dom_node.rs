use browser_proto::{DomAttr, DomNode};
use hotmeal::Document;

/// Convert hotmeal's Document body to DomNode tree for comparison with browser.
///
/// For full document parsing, this returns the `<body>` element.
/// For fragment parsing (where root IS the body context), this returns the root.
pub fn document_body_to_dom_node(doc: &Document) -> Option<DomNode> {
    // Try to find <body> first (full document parsing)
    // If not found, use root directly (fragment parsing where root is the context element)
    let body = doc.body().unwrap_or(doc.root);
    Some(node_to_dom_node(doc, body))
}

fn node_to_dom_node(doc: &Document, node_id: hotmeal::NodeId) -> DomNode {
    let node = doc.get(node_id);
    match &node.kind {
        hotmeal::NodeKind::Element(elem) => {
            let tag = elem.tag.to_string().to_ascii_lowercase();
            let mut attrs: Vec<DomAttr> = elem
                .attrs
                .iter()
                .map(|(qname, value)| DomAttr {
                    name: qname.local.to_string(),
                    value: value.as_ref().to_string(),
                })
                .collect();
            attrs.sort_by(|a, b| a.name.cmp(&b.name));
            let children: Vec<DomNode> = doc
                .children(node_id)
                .map(|child_id| node_to_dom_node(doc, child_id))
                .collect();
            DomNode::Element {
                tag,
                attrs,
                children,
            }
        }
        hotmeal::NodeKind::Text(text) => DomNode::Text(text.as_ref().to_string()),
        hotmeal::NodeKind::Comment(text) => DomNode::Comment(text.as_ref().to_string()),
        hotmeal::NodeKind::Document => DomNode::Text(String::new()),
    }
}
