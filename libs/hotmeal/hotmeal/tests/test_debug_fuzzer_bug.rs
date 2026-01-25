//! Debug the specific fuzzer bug

use facet_testhelpers::test;
use hotmeal::{NodePath, NodeRef, Patch, StrTendril};
#[allow(unused_imports)]
use hotmeal::{debug, trace};
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
    trace!(?result, "Result");
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
    trace!(?result, "Result");
    trace!(html = %doc.to_html(), "HTML");
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
    trace!(?patches, "Patches");

    old.apply_patches(patches).expect("apply failed");
    let result = old.to_html();
    trace!(result = %result, "Result");

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

/// Regression test for textarea content corruption
/// html_a: "\n$"
/// html_b: "<textarea>\n\n\n\n" (unclosed - browser captures </body></html> as content)
#[test]
fn test_textarea_content() {
    // Simulate what the browser does: unclosed textarea captures the closing tags
    let a = t("<html><body>\n$</body></html>");
    // Browser normalizes unclosed <textarea>\n\n\n\n</body></html> to this:
    // (the </body></html> become escaped text inside textarea)
    let b = t("<html><body><textarea>\n\n\n\n&lt;/body&gt;&lt;/html&gt;</textarea></body></html>");

    trace!(?a, "Parsing a");
    let doc_a = hotmeal::parse(&a);
    trace!(doc_a_html = %doc_a.to_html(), "doc_a HTML");

    trace!(?b, "Parsing b");
    let doc_b = hotmeal::parse(&b);
    trace!(doc_b_html = %doc_b.to_html(), "doc_b HTML");

    trace!("Computing diff...");
    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff should succeed");
    trace!(?patches, "Patches");

    // Apply patches
    let mut patched = doc_a.clone();
    patched
        .apply_patches(patches)
        .expect("apply should succeed");

    trace!(patched = %patched.to_html(), "Patched");
    trace!(expected = %doc_b.to_html(), "Expected");

    assert_eq!(
        patched.to_html(),
        doc_b.to_html(),
        "Patched HTML should match target"
    );
}

/// Regression test for fuzzer crash with <s = input
/// html_a: "'" (single quote)
/// html_b: "<s ="
#[test]
fn test_text_to_s_element() {
    let a = t("<html><body>'</body></html>");
    let b = t("<html><body><s =</body></html>");

    trace!(?a, "Parsing a");
    let doc_a = hotmeal::parse(&a);
    trace!(doc_a_html = %doc_a.to_html(), "doc_a HTML");

    trace!(?b, "Parsing b");
    let doc_b = hotmeal::parse(&b);
    trace!(doc_b_html = %doc_b.to_html(), "doc_b HTML");

    trace!("Computing diff...");
    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff should succeed");
    trace!(?patches, "Patches");

    // Apply patches
    let mut patched = doc_a.clone();
    patched
        .apply_patches(patches)
        .expect("apply should succeed");

    trace!(patched = %patched.to_html(), "Patched");
    trace!(expected = %doc_b.to_html(), "Expected");

    assert_eq!(
        patched.to_html(),
        doc_b.to_html(),
        "Patched HTML should match target"
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

    trace!(?a, "Parsing a");
    let doc_a = hotmeal::parse(&a);
    trace!(doc_a_html = %doc_a.to_html(), "doc_a HTML");
    trace!(doc_a_body = ?doc_a.body(), "doc_a.body()");

    trace!(?b, "Parsing b");
    let doc_b = hotmeal::parse(&b);
    trace!(doc_b_html = %doc_b.to_html(), "doc_b HTML");
    trace!(doc_b_body = ?doc_b.body(), "doc_b.body()");

    // Both docs may have no body, but diff should still work
    trace!("Computing diff...");
    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff should succeed");
    trace!(?patches, "Patches");

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
