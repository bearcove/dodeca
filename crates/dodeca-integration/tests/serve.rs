//! Integration tests for the serve functionality
//!
//! These tests verify that all pages are accessible when serving a site.

use dodeca_integration::TestSite;
use std::time::Duration;

/// Test that all expected pages return 200
#[test_log::test]
fn test_all_pages_accessible() {
    let site = TestSite::new("sample-site");

    // Test all expected pages
    site.get("/").assert_ok();
    site.get("/guide/").assert_ok();
    site.get("/guide/getting-started/").assert_ok();
    site.get("/guide/advanced/").assert_ok();
}

/// Test that CSS files have their internal URLs rewritten to cache-busted versions
#[test_log::test]
fn test_css_font_urls_are_rewritten() {
    let site = TestSite::new("sample-site");

    // Fetch the HTML and find the CSS URL
    let html = site.get("/");
    html.assert_ok();

    // Find the cache-busted CSS URL in HTML
    let css_url = html
        .css_link("/css/style.*.css")
        .expect("Could not find cache-busted CSS URL in HTML");

    // Fetch the CSS file
    let css = site.get(&css_url);
    css.assert_ok();

    // Verify font URLs in CSS are cache-busted (not the original /fonts/test.woff2)
    css.assert_not_contains("/fonts/test.woff2");

    // Font URL should be cache-busted
    assert!(
        css.text().contains("/fonts/test."),
        "CSS should contain cache-busted font URL"
    );
}

/// Test that CSS changes are picked up by the file watcher (livereload)
#[test_log::test]
fn test_css_livereload() {
    let site = TestSite::new("sample-site");

    // Fetch the HTML and find the CSS URL
    let html = site.get("/");
    html.assert_ok();

    let css_url_1 = html
        .css_link("/css/style.*.css")
        .expect("Could not find CSS URL in HTML");

    // Fetch the initial CSS content
    let css_1 = site.get(&css_url_1);
    css_1.assert_ok();
    assert!(
        css_1.text().contains("font-weight: 400") || css_1.text().contains("font-weight:400"),
        "Initial CSS should contain font-weight: 400"
    );

    // Give the watcher debounce window time to clear
    site.wait_debounce();

    // Modify the CSS file
    site.modify_file("static/css/style.css", |content| {
        content.replace("font-weight: 400", "font-weight: 700")
    });

    // Wait for the change to be picked up
    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let html = site.get("/");
        let new_url = html.css_link("/css/style.*.css")?;
        if new_url != css_url_1 {
            Some(new_url)
        } else {
            None
        }
    });

    // Fetch the new CSS content
    let css_2 = site.get(&css_url_2);
    css_2.assert_ok();
    assert!(
        css_2.text().contains("font-weight: 700") || css_2.text().contains("font-weight:700"),
        "Updated CSS should contain font-weight: 700"
    );
}

/// Test that new content files are detected and served
#[test_log::test]
fn test_new_content_file_detected() {
    let site = TestSite::new("sample-site");

    // Verify the new page doesn't exist yet
    site.get("/new-page/").assert_not_found();

    // Create the new page
    site.write_file(
        "content/new-page.md",
        r#"+++
title = "New Page"
+++

This is a dynamically created page.
"#,
    );

    // Wait for file watcher to pick up the change
    let resp = site.wait_for("/new-page/", Duration::from_secs(10));
    resp.assert_ok()
        .assert_contains("dynamically created");
}

/// Test that deleted content files result in 404
#[test_log::test]
fn test_deleted_content_file_returns_404() {
    let site = TestSite::new("sample-site");

    // Create a page
    site.write_file(
        "content/temp-page.md",
        r#"+++
title = "Temporary Page"
+++

This page will be deleted.
"#,
    );

    // Wait for the page to be available
    site.wait_for("/temp-page/", Duration::from_secs(10));

    // Delete the page
    site.delete_file("content/temp-page.md");

    // Wait for the page to return 404
    site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/temp-page/");
        if resp.status == 404 {
            Some(())
        } else {
            None
        }
    });
}
