//! Template rendering tests

use crate::harness::TestSite;

#[test_log::test]
fn template_renders_content() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();

    // Template should wrap content in proper HTML structure
    // Note: minified HTML uses lowercase doctype
    html.assert_contains("<!doctype html>");
    html.assert_contains("<body>");
}

#[test_log::test]
fn template_includes_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // Template should include CSS links
    assert!(
        html.text().contains("stylesheet") || html.text().contains(".css"),
        "Template should include CSS references"
    );
}

#[test_log::test]
fn template_metadata_used() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // Frontmatter title should be used in <title> tag
    html.assert_contains("<title>Home</title>");
}

#[test_log::test]
fn different_templates_for_different_pages() {
    let site = TestSite::new("sample-site");

    // Both pages should render successfully
    let index = site.get("/");
    let guide = site.get("/guide/");

    index.assert_ok();
    guide.assert_ok();

    // Both should have proper HTML structure (minified)
    index.assert_contains("<!doctype html>");
    guide.assert_contains("<!doctype html>");
}
