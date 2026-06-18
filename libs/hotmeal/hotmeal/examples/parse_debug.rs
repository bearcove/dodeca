use hotmeal::{NodeKind, StrTendril};
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();

    let tendril = StrTendril::from(input);
    let doc = hotmeal::parse_body_fragment(&tendril);

    fn print_node(doc: &hotmeal::Document, node_id: hotmeal::NodeId, depth: usize) {
        let indent = "  ".repeat(depth);
        let node = doc.get(node_id);
        match &node.kind {
            NodeKind::Element(el) => {
                println!("{}<{}>", indent, el.tag);
                for child_id in node_id.children(&doc.arena) {
                    print_node(doc, child_id, depth + 1);
                }
                println!("{}</{}>", indent, el.tag);
            }
            NodeKind::Text(t) => {
                if !t.trim().is_empty() {
                    println!("{}TEXT: {:?}", indent, t.as_ref());
                }
            }
            NodeKind::Comment(c) => {
                println!("{}COMMENT: {:?}", indent, c.as_ref());
            }
            NodeKind::Document => {
                for child_id in node_id.children(&doc.arena) {
                    print_node(doc, child_id, depth);
                }
            }
        }
    }

    if let Some(body_id) = doc.body() {
        print_node(&doc, body_id, 0);
    } else {
        eprintln!("No body found, printing from root");
        print_node(&doc, doc.root, 0);
    }

    // Also print max depth
    fn max_depth(doc: &hotmeal::Document, node_id: hotmeal::NodeId) -> usize {
        let children_depth = node_id
            .children(&doc.arena)
            .map(|c| max_depth(doc, c))
            .max()
            .unwrap_or(0);
        let node = doc.get(node_id);
        match &node.kind {
            NodeKind::Element(_) => 1 + children_depth,
            NodeKind::Document => children_depth,
            _ => 0,
        }
    }

    if let Some(body_id) = doc.body() {
        eprintln!("\nMax depth: {}", max_depth(&doc, body_id));
    }
}
