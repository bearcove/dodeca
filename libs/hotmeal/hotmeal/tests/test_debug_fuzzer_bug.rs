//! Debug the specific fuzzer bug

use facet_testhelpers::test;
use hotmeal::{NodePath, NodeRef, Patch, StrTendril};
use html5ever::LocalName;
use smallvec::smallvec;

/// Helper to create a StrTendril from a string
fn t(s: &str) -> StrTendril {
    StrTendril::from(s)
}

#[test]
fn test_minimal_repro() {
    // Start with simple structure
    let html = t("<html><body><div></div></body></html>");
    let mut doc = hotmeal::parse(&html);

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
    let html = t("<html><body><div>1</div><div>2</div></body></html>");
    let mut doc = hotmeal::parse(&html);

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
    let old_html = t(r#"<html><body><img><img src="" alt=""><img src=""></body></html>"#);
    let new_html = t(r#"<html><body><img src="" alt=""><img src=""></body></html>"#);

    let mut old = hotmeal::parse(&old_html);
    let new = hotmeal::parse(&new_html);

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

/// Regression test for fuzzer crash with minimal input
/// crash-9d5bce5f90d295276df5f9cfa00700d855d43ad8, minimized to 4 bytes
/// html_a: "<!" (bytes [60, 33])
/// html_b: "" (empty)
///
/// The issue: "<!" parses to just a comment (no body), but diff() crashed.
/// Now diff() handles missing bodies gracefully.
#[test]
fn test_diff_incomplete_doctype_vs_empty() {
    let a = t("<!");
    let b = t("");

    println!("Parsing a: {:?}", a);
    let doc_a = hotmeal::parse(&a);
    println!("doc_a HTML: {:?}", doc_a.to_html());
    println!("doc_a.body(): {:?}", doc_a.body());

    println!("Parsing b: {:?}", b);
    let doc_b = hotmeal::parse(&b);
    println!("doc_b HTML: {:?}", doc_b.to_html());
    println!("doc_b.body(): {:?}", doc_b.body());

    // Both docs may have no body, but diff should still work
    println!("Computing diff...");
    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff should succeed");
    println!("Patches: {:?}", patches);

    // Apply patches
    let mut patched = doc_a.clone();
    patched
        .apply_patches(patches)
        .expect("apply should succeed");

    // Compare body contents - the diff system only operates on body content.
    // Both "no body" and "empty body" produce empty body content.
    assert_eq!(
        patched.to_body_html(),
        doc_b.to_body_html(),
        "Patched body content should match target body content"
    );
}
