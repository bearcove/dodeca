//! Debug the specific fuzzer bug

use facet_testhelpers::test;
use hotmeal::diff::{NodePath, NodeRef, Patch};

#[test]
fn test_minimal_repro() {
    // Start with simple structure
    let mut doc = hotmeal::arena_dom::parse("<html><body><div></div></body></html>");

    // Try to insert at position 1 when there's only 1 child
    let patches = vec![Patch::InsertElement {
        at: NodeRef::Path(NodePath(vec![1])),
        tag: "p".to_string(),
        attrs: vec![],
        children: vec![],
        detach_to_slot: None,
    }];

    let result = doc.apply_patches(&patches);
    println!("Result: {:?}", result);
    assert!(result.is_ok(), "should be able to insert at end");
}

#[test]
fn test_move_within_same_parent() {
    // Test moving within same parent
    let mut doc = hotmeal::arena_dom::parse("<html><body><div>1</div><div>2</div></body></html>");

    // Move first child to position 1 (swap them)
    let patches = vec![Patch::Move {
        from: NodeRef::Path(NodePath(vec![0])),
        to: NodeRef::Path(NodePath(vec![1])),
        detach_to_slot: None,
    }];

    let result = doc.apply_patches(&patches);
    println!("Result: {:?}", result);
    println!("HTML: {}", doc.to_html());
}
