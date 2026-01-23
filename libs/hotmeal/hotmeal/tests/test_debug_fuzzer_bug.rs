//! Debug the specific fuzzer bug

use facet_testhelpers::test;
use hotmeal::{NodePath, NodeRef, Patch};
use html5ever::LocalName;

#[test]
fn test_minimal_repro() {
    // Start with simple structure
    let mut doc = hotmeal::parse("<html><body><div></div></body></html>");

    // Try to insert at position 1 when there's only 1 child
    // Path: [slot=0, position=1] - insert at second position in body
    let patches = vec![Patch::InsertElement {
        at: NodeRef(NodePath(vec![0, 1])),
        tag: LocalName::from("p"),
        attrs: vec![],
        children: vec![],
        detach_to_slot: None,
    }];

    let result = doc.apply_patches(patches);
    println!("Result: {:?}", result);
    assert!(result.is_ok(), "should be able to insert at end");
}

#[test]
fn test_move_within_same_parent() {
    // Test moving within same parent
    let mut doc = hotmeal::parse("<html><body><div>1</div><div>2</div></body></html>");

    // Move first child to position 1 (swap them)
    // Paths: [slot=0, position=X] - within body
    let patches = vec![Patch::Move {
        from: NodeRef(NodePath(vec![0, 0])),
        to: NodeRef(NodePath(vec![0, 1])),
        detach_to_slot: None,
    }];

    let result = doc.apply_patches(patches);
    println!("Result: {:?}", result);
    println!("HTML: {}", doc.to_html());
}
