//! Tests for HTML diff path translation.

use dodeca_protocol::{NodePath, Patch};
use facet_testhelpers::test;

#[test]
fn test_simple_text_change() {
    let old = r#"<html><body><p>Hello</p></body></html>"#;
    let new = r#"<html><body><p>Goodbye</p></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0],
        Patch::SetText {
            path: NodePath(vec![0]),
            text: "Goodbye".to_string(),
        }
    );
}

#[test]
fn test_insert_element() {
    let old = r#"<html><body><p>First</p></body></html>"#;
    let new = r#"<html><body><p>First</p><p>Second</p></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    // Should insert the second paragraph
    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::InsertBefore { path, html } | Patch::AppendChild { path, html }
            if html.contains("Second")
        )),
        "Expected insert patch for 'Second', got: {:?}",
        patches
    );
}

#[test]
fn test_remove_element() {
    let old = r#"<html><body><p>First</p><p>Second</p></body></html>"#;
    let new = r#"<html><body><p>First</p></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    assert!(
        patches.iter().any(|p| matches!(p, Patch::Remove { path } if path.0 == vec![1])),
        "Expected Remove at path [1], got: {:?}",
        patches
    );
}

#[test]
fn test_attribute_change() {
    let old = r#"<html><body><div class="old">Content</div></body></html>"#;
    let new = r#"<html><body><div class="new">Content</div></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetAttribute { path, name, value }
            if path.0 == vec![0] && name == "class" && value == "new"
        )),
        "Expected SetAttribute for class='new', got: {:?}",
        patches
    );
}

#[test]
fn test_mixed_changes() {
    let old = r#"<html><body><div class="box"><p>One</p><p>Two</p></div></body></html>"#;
    let new = r#"<html><body><div class="container"><p>One</p><p>Modified</p><p>Three</p></div></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    // Should have attribute change
    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetAttribute { name, value, .. }
            if name == "class" && value == "container"
        )),
        "Expected SetAttribute for class='container', got: {:?}",
        patches
    );

    // Should have some patch for "Modified" (could be SetText or InsertBefore depending on diff algorithm)
    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetText { text, .. } if text == "Modified"
        ) || matches!(p,
            Patch::InsertBefore { html, .. } | Patch::AppendChild { html, .. }
            if html.contains("Modified")
        )),
        "Expected patch for 'Modified', got: {:?}",
        patches
    );

    // Should have some patch for "Three" (SetText or Insert)
    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetText { text, .. } if text == "Three"
        ) || matches!(p,
            Patch::InsertBefore { html, .. } | Patch::AppendChild { html, .. }
            if html.contains("Three")
        )),
        "Expected patch for 'Three', got: {:?}",
        patches
    );
}

#[test]
fn test_nested_text_change() {
    let old = r#"<html><body><div><span>Hello</span></div></body></html>"#;
    let new = r#"<html><body><div><span>World</span></div></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetText { text, .. } if text == "World"
        )),
        "Expected SetText for 'World', got: {:?}",
        patches
    );
}

#[test]
fn test_add_attribute() {
    let old = r#"<html><body><div>Content</div></body></html>"#;
    let new = r#"<html><body><div id="main">Content</div></body></html>"#;

    let patches = html_diff_tests::diff_html(old, new).unwrap();

    assert!(
        patches.iter().any(|p| matches!(p,
            Patch::SetAttribute { name, value, .. }
            if name == "id" && value == "main"
        )),
        "Expected SetAttribute for id='main', got: {:?}",
        patches
    );
}

#[test]
fn test_identical_documents() {
    let html = r#"<html><body><p>Same content</p></body></html>"#;

    let patches = html_diff_tests::diff_html(html, html).unwrap();

    assert!(
        patches.is_empty(),
        "Expected no patches for identical documents, got: {:?}",
        patches
    );
}
