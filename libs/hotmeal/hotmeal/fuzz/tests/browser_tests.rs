//! Browser-based tests using thrall.
//!
//! These tests launch Chrome and verify that hotmeal patches work correctly
//! when applied via the hotmeal-wasm module in a real browser.

/// Test that moving an ancestor under its descendant works correctly in the browser.
///
/// This is a regression test for a bug where the cycle prevention fix in the
/// native diff code didn't translate correctly to the browser-side patch application.
#[test]
fn test_move_ancestor_under_descendant_browser() {
    facet_testhelpers::setup();

    // Original: body > footer > div > span
    // New: body > div > span > footer (footer moves under its former descendant)
    let old_html = r#"<!DOCTYPE html><html><body><footer><div><span>content</span></div></footer></body></html>"#;
    let new_html = r#"<!DOCTYPE html><html><body><div><span>content<footer></footer></span></div></body></html>"#;

    let result = thrall::test_roundtrip(old_html.to_string(), new_html.to_string());
    let result = result.expect("browser connection failed");

    // The final DOM should match the expected normalized new HTML
    assert_eq!(
        result.result_html, result.normalized_new,
        "Roundtrip failed!\nExpected: {}\nGot: {}\nPatches applied: {}",
        result.normalized_new, result.result_html, result.patch_count
    );
}

/// Test deeply nested footer elements that triggered the original OOM bug.
#[test]
fn test_deeply_nested_footers_browser() {
    facet_testhelpers::setup();

    // This is a simplified version of the fuzzer-generated input that caused OOM
    let old_html = r#"<!DOCTYPE html><html><body><footer><footer><footer><footer></footer></footer></footer></footer></body></html>"#;
    let new_html = r#"<!DOCTYPE html><html><body><footer><footer><footer></footer></footer></footer></body></html>"#;

    let result = thrall::test_roundtrip(old_html.to_string(), new_html.to_string());
    let result = result.expect("browser connection failed");

    assert_eq!(
        result.result_html, result.normalized_new,
        "Roundtrip failed!\nExpected: {}\nGot: {}\nPatches applied: {}",
        result.normalized_new, result.result_html, result.patch_count
    );
}

/// Simple sanity check that browser roundtrip works at all.
#[test]
fn test_simple_text_change_browser() {
    facet_testhelpers::setup();

    let old_html = r#"<!DOCTYPE html><html><body><p>Hello</p></body></html>"#;
    let new_html = r#"<!DOCTYPE html><html><body><p>World</p></body></html>"#;

    let result = thrall::test_roundtrip(old_html.to_string(), new_html.to_string());
    let result = result.expect("browser connection failed");

    assert_eq!(
        result.result_html, result.normalized_new,
        "Simple text change failed!\nExpected: {}\nGot: {}",
        result.normalized_new, result.result_html
    );
}

/// Test adding an element.
#[test]
fn test_add_element_browser() {
    facet_testhelpers::setup();

    let old_html = r#"<!DOCTYPE html><html><body><div></div></body></html>"#;
    let new_html = r#"<!DOCTYPE html><html><body><div><p>New paragraph</p></div></body></html>"#;

    let result = thrall::test_roundtrip(old_html.to_string(), new_html.to_string());
    let result = result.expect("browser connection failed");

    assert_eq!(
        result.result_html, result.normalized_new,
        "Add element failed!\nExpected: {}\nGot: {}",
        result.normalized_new, result.result_html
    );
}

/// Test removing an element.
#[test]
fn test_remove_element_browser() {
    facet_testhelpers::setup();

    let old_html = r#"<!DOCTYPE html><html><body><div><p>To be removed</p></div></body></html>"#;
    let new_html = r#"<!DOCTYPE html><html><body><div></div></body></html>"#;

    let result = thrall::test_roundtrip(old_html.to_string(), new_html.to_string());
    let result = result.expect("browser connection failed");

    assert_eq!(
        result.result_html, result.normalized_new,
        "Remove element failed!\nExpected: {}\nGot: {}",
        result.normalized_new, result.result_html
    );
}
