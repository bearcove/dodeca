//! Template error detection tests
//!
//! Tests that template errors are properly detected and shown to the user
//! during development.

use crate::harness::TestSite;
use std::time::Duration;

/// Error marker that should be present in error pages
const RENDER_ERROR_MARKER: &str = "<!-- DODECA_RENDER_ERROR -->";

#[test_log::test]
fn template_syntax_error_shows_error_page() {
    let site = TestSite::new("sample-site");

    // Verify the page works initially
    let html = site.get("/");
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");

    // Introduce a template syntax error: unclosed tag
    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    // Wait for file watcher to pick up the change
    std::thread::sleep(Duration::from_millis(500));

    // Request the page again - should show error page
    let html = site.get("/");
    html.assert_ok(); // Still returns 200 (error page is valid HTML)
    html.assert_contains(RENDER_ERROR_MARKER);
    html.assert_contains("Template Error"); // Error page title
}

#[test_log::test]
fn template_error_recovery_removes_error_page() {
    let site = TestSite::new("sample-site");

    // Save the original template
    let original = site.read_file("templates/index.html");

    // Introduce a template syntax error
    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    // Wait for file watcher
    std::thread::sleep(Duration::from_millis(500));

    // Verify error page is shown
    let html = site.get("/");
    html.assert_contains(RENDER_ERROR_MARKER);

    // Fix the template
    site.write_file("templates/index.html", &original);

    // Wait for file watcher
    std::thread::sleep(Duration::from_millis(500));

    // Verify page works again
    let html = site.get("/");
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");
}

#[test_log::test]
fn missing_template_shows_error_page() {
    let site = TestSite::new("sample-site");

    // Verify the page works initially
    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);

    // Delete the section template
    site.delete_file("templates/section.html");

    // Wait for file watcher
    std::thread::sleep(Duration::from_millis(500));

    // Request the section page - should show error
    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
}
