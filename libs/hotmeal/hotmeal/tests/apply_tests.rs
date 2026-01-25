//! Tests for patch application.

use facet_testhelpers::test;
#[allow(unused_imports)]
use hotmeal::{debug, trace};

use hotmeal::{
    AttrPair, InsertContent, NodeKind, NodePath, NodeRef, Patch, Stem, StrTendril, parse,
};
use html5ever::{LocalName, QualName, local_name, ns};
use smallvec::smallvec;

/// Helper to create a StrTendril from a string
fn t(s: &str) -> StrTendril {
    StrTendril::from(s)
}

#[test]
fn test_parse_and_serialize_roundtrip() {
    let html = t("<html><body><p>Hello</p></body></html>");
    let node = parse(&html);
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>Hello</p></body></html>"
    );
}

#[test]
fn test_apply_set_text() {
    let html = t("<html><body><p>Hello</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, p=0, text=0] - first child of body is p, first child of p is text
    node.apply_patches(vec![Patch::SetText {
        path: NodePath(smallvec![0, 0, 0]),
        text: Stem::from("Goodbye"),
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>Goodbye</p></body></html>"
    );
}

#[test]
fn test_apply_set_attribute() {
    let html = t("<html><body><div>Content</div></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, div=0] - first child of body is the div
    node.apply_patches(vec![Patch::SetAttribute {
        path: NodePath(smallvec![0, 0]),
        name: QualName::new(None, ns!(), local_name!("class")),
        value: Stem::from("highlight"),
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><div class=\"highlight\">Content</div></body></html>"
    );
}

#[test]
fn test_apply_remove() {
    let html = t("<html><body><p>First</p><p>Second</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=1] - remove second child of body
    node.apply_patches(vec![Patch::Remove {
        node: NodeRef(NodePath(smallvec![0, 1])),
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>First</p></body></html>"
    );
}

#[test]
fn test_apply_insert_element() {
    let html = t("<html><body><p>First</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=0] - insert at first position in body, displacing to slot 1
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef(NodePath(smallvec![0, 0])),
        tag: LocalName::from("p"),
        attrs: vec![],
        children: vec![],
        detach_to_slot: Some(1),
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p></p></body></html>"
    );
}

#[test]
fn test_apply_insert_element_no_displacement() {
    let html = t("<html><body><p>First</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=1] - insert at second position (no displacement)
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef(NodePath(smallvec![0, 1])),
        tag: LocalName::from("p"),
        attrs: vec![],
        children: vec![],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>First</p><p></p></body></html>"
    );
}

#[test]
fn test_apply_insert_element_with_children() {
    let html = t("<html><body><p>First</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=1] - insert at second position
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef(NodePath(smallvec![0, 1])),
        tag: LocalName::from("p"),
        attrs: vec![],
        children: vec![InsertContent::Text(Stem::from("Second"))],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>First</p><p>Second</p></body></html>"
    );
}

#[test]
fn test_apply_insert_element_with_attrs() {
    let html = t("<html><body><p>First</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=1] - insert at second position
    node.apply_patches(vec![Patch::InsertElement {
        at: NodeRef(NodePath(smallvec![0, 1])),
        tag: LocalName::from("p"),
        attrs: vec![AttrPair {
            name: QualName::new(None, ns!(), local_name!("class")),
            value: Stem::from("highlight"),
        }],
        children: vec![InsertContent::Text(Stem::from("Second"))],
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>First</p><p class=\"highlight\">Second</p></body></html>"
    );
}

#[test]
fn test_apply_insert_text() {
    let html = t("<html><body><p>First</p></body></html>");
    let mut node = parse(&html);
    // Path: [slot=0, position=1] - insert text at second position
    node.apply_patches(vec![Patch::InsertText {
        at: NodeRef(NodePath(smallvec![0, 1])),
        text: Stem::from("Hello"),
        detach_to_slot: None,
    }])
    .unwrap();
    assert_eq!(
        node.to_html(),
        "<html><head></head><body><p>First</p>Hello</body></html>"
    );
}

#[test]
fn test_parse_invalid_html_nesting() {
    let html = t(r#"<html><body><strong><div>nested div</div></strong></body></html>"#);
    let doc = parse(&html);

    let body = doc.body().expect("should have body");
    let strong = doc.first_child(body).expect("body should have children");

    if let NodeKind::Element(elem) = &doc.get(strong).kind {
        assert_eq!(elem.tag.as_ref(), "strong");
    } else {
        panic!("Expected element, got {:?}", doc.get(strong).kind);
    }

    let div = doc
        .first_child(strong)
        .expect("strong should have children");
    if let NodeKind::Element(elem) = &doc.get(div).kind {
        assert_eq!(elem.tag.as_ref(), "div");
    } else {
        panic!("Expected div element, got {:?}", doc.get(div).kind);
    }

    let text_node = doc.first_child(div).expect("div should have text");
    if let NodeKind::Text(text) = &doc.get(text_node).kind {
        assert_eq!(text.as_ref(), "nested div");
    } else {
        panic!("Expected text node");
    }
}
