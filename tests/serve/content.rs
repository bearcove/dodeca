//! Content file detection and rendering tests

use crate::harness::TestSite;

#[test_log::test]
fn markdown_content_rendered() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();
    // Content from _index.md should be rendered
    html.assert_contains("Welcome");
    html.assert_contains("This is the home page");
}

#[test_log::test]
fn frontmatter_title_in_html() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    // Title from frontmatter should appear in <title>
    html.assert_contains("<title>Home</title>");
}

#[test_log::test]
fn nested_content_structure() {
    let site = TestSite::new("sample-site");

    // Nested pages should be accessible
    site.get("/guide/").assert_ok();
    site.get("/guide/getting-started/").assert_ok();
    site.get("/guide/advanced/").assert_ok();
}
