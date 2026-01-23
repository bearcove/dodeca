//! Debug the specific fuzzer bug

use facet_testhelpers::test;
use hotmeal::{NodePath, NodeRef, Patch};
use html5ever::LocalName;
use smallvec::smallvec;

#[test]
fn test_minimal_repro() {
    // Start with simple structure
    let mut doc = hotmeal::parse("<html><body><div></div></body></html>");

    // Try to insert at position 1 when there's only 1 child
    // Path: [slot=0, position=1] - insert at second position in body
    let patches = vec![Patch::InsertElement {
        at: NodeRef(NodePath(smallvec![0, 1])),
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
        from: NodeRef(NodePath(smallvec![0, 0])),
        to: NodeRef(NodePath(smallvec![0, 1])),
        detach_to_slot: None,
    }];

    let result = doc.apply_patches(patches);
    println!("Result: {:?}", result);
    println!("HTML: {}", doc.to_html());
}

/// Regression test for fuzzer crash-f9f4d0f90a4824d024b1aedd19e5ac64afefed5f
/// Issue: img elements losing/gaining alt attribute incorrectly
#[test]
fn test_img_alt_attribute_preservation() {
    // Old has: <img> (no attrs), <img src="" alt="">, <img src=""> (no alt)
    // New has: <img src="" alt="">, <img src=""> (no alt)
    // The second img should NOT get alt=""
    let old_html = r#"<html><body><img><img src="" alt=""><img src=""></body></html>"#;
    let new_html = r#"<html><body><img src="" alt=""><img src=""></body></html>"#;

    let mut old = hotmeal::parse(old_html);
    let new = hotmeal::parse(new_html);

    let patches = hotmeal::diff(&old, &new).expect("diff failed");
    println!("Patches: {:#?}", patches);

    old.apply_patches(patches).expect("apply failed");
    let result = old.to_html();
    println!("Result: {}", result);

    // The second img should have src="" but NOT alt=""
    assert!(
        !result.contains(r#"<img src="" alt=""><img src="" alt="">"#),
        "Second img should not have alt attribute, got: {}",
        result
    );
    assert!(
        result.contains(r#"<img src="" alt=""><img src="">"#),
        "Expected <img src=\"\" alt=\"\"><img src=\"\">, got: {}",
        result
    );
}
