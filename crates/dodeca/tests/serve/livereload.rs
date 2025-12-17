//! File watching and live reload tests
//!
//! Tests for detecting file changes and serving updated content.

use crate::harness::TestSite;
use std::time::Duration;

/// Test that new section files (_index.md) are detected
/// This tests that newly created subdirectories are watched (issue #9)
#[test_log::test]
fn test_new_section_detected() {
    let site = TestSite::new("sample-site");

    // Wait for file watcher to fully initialize after server startup
    site.wait_debounce();

    // Ensure the new section doesn't exist initially
    site.delete_if_exists("content/new-section");

    // Verify the new section doesn't exist yet
    eprintln!("TEST: About to GET /new-section/ expecting 404...");
    let resp = site.get("/new-section/");
    eprintln!("TEST: Got status {} for /new-section/", resp.status);
    assert_eq!(resp.status, 404, "New section should not exist initially");

    // Create the new section
    site.write_file(
        "content/new-section/_index.md",
        r#"+++
title = "New Section"
+++

This is a dynamically created section."#,
    );

    // Poll until the new section is accessible (up to 10s)
    let _resp = site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/new-section/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    // Verify content
    let resp = site.get("/new-section/");
    resp.assert_ok();
    resp.assert_contains("dynamically created section");
}

/// Test that deeply nested new directories with files are detected
/// This tests recursive scanning of newly created directory trees
#[test_log::test]
fn test_deeply_nested_new_section() {
    let site = TestSite::new("sample-site");

    // Wait for file watcher to fully initialize after server startup
    site.wait_debounce();

    // Ensure the nested directories don't exist before starting
    site.delete_if_exists("content/level1");

    // Verify the deeply nested section doesn't exist yet
    let resp = site.get("/level1/level2/level3/");
    assert_eq!(
        resp.status, 404,
        "Nested section should not exist initially"
    );

    // Create the entire nested directory structure
    site.write_file(
        "content/level1/level2/level3/_index.md",
        r#"+++
title = "Deeply Nested"
+++

This is a deeply nested section at level 3."#,
    );

    // Poll until the nested section is accessible (up to 10s)
    let _resp = site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/level1/level2/level3/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    // Verify content
    let resp = site.get("/level1/level2/level3/");
    resp.assert_ok();
    resp.assert_contains("deeply nested section");
}

/// Test that file moves (renames) are detected correctly (issue #10)
/// When a file is moved, the old route should 404 and the new route should work
#[test_log::test]
fn test_file_move_detected() {
    let site = TestSite::new("sample-site");

    // Wait for file watcher to fully initialize after server startup
    site.wait_debounce();

    // Create a page in the 'guide' section first
    site.write_file(
        "content/guide/moveable.md",
        r#"+++
title = "Moveable Page"
+++

This page will be moved."#,
    );

    // Wait for the page to be accessible at the original location
    site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/guide/moveable/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    site.wait_debounce();

    // Move the file to a different location
    let original_content = site.read_file("content/guide/moveable.md");
    site.delete_file("content/guide/moveable.md");
    site.write_file("content/moved-page.md", &original_content);

    // Poll until the new location is accessible AND the old location returns 404
    let result = site.wait_until(Duration::from_secs(10), || {
        let old_resp = site.get("/guide/moveable/");
        let new_resp = site.get("/moved-page/");

        if old_resp.status == 404 && new_resp.status == 200 {
            Some((old_resp, new_resp))
        } else {
            None
        }
    });

    let (old_resp, new_resp) = result;
    assert_eq!(
        old_resp.status, 404,
        "Old URL should return 404 after file move"
    );
    assert_eq!(
        new_resp.status, 200,
        "New URL should be accessible after file move"
    );
    new_resp.assert_contains("This page will be moved");
}

/// Test that CSS changes are picked up by the file watcher (livereload)
#[test_log::test]
fn test_css_livereload() {
    let site = TestSite::new("sample-site");

    // Ensure baseline CSS content
    const BASELINE_CSS: &str = r#"/* Test CSS with font URLs */
@font-face {
    font-family: 'TestFont';
    src: url('/fonts/test.woff2') format('woff2');
    font-weight: 400;
    font-style: normal;
}

body {
    font-family: 'TestFont', sans-serif;
}
"#;
    site.write_file("static/css/style.css", BASELINE_CSS);

    // Get initial CSS URL
    let css_url_1 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("Initial CSS URL should exist");

    // Verify initial CSS content (minified, so no space after colon)
    let css_1 = site.get(&css_url_1);
    css_1.assert_contains("font-weight:400");

    site.wait_debounce();

    // Modify the CSS file (source has space after colon)
    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    // Poll until file watcher reloads (up to 10s)
    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let new_url = site.get("/").css_link("/css/style.*.css")?;
        if new_url != css_url_1 {
            Some(new_url)
        } else {
            None
        }
    });

    // Verify new CSS content (minified)
    let css_2 = site.get(&css_url_2);
    css_2.assert_contains("font-weight:700");
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification"
    );
}

/// Test that CSS changes are picked up in TUI mode (livereload)
/// NOTE: Requires real PTY - ignored until we add portable-pty testing
#[test_log::test]
#[ignore = "TUI mode requires real terminal, needs PTY testing setup"]
fn test_css_livereload_tui_mode() {
    let site = TestSite::new("sample-site");

    // Ensure baseline CSS content
    const BASELINE_CSS: &str = r#"/* Test CSS with font URLs */
@font-face {
    font-family: 'TestFont';
    src: url('/fonts/test.woff2') format('woff2');
    font-weight: 400;
    font-style: normal;
}

body {
    font-family: 'TestFont', sans-serif;
}
"#;
    site.write_file("static/css/style.css", BASELINE_CSS);

    // Get initial CSS URL
    let css_url_1 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("Initial CSS URL should exist");

    // Verify initial CSS content (minified, so no space after colon)
    let css_1 = site.get(&css_url_1);
    css_1.assert_contains("font-weight:400");

    // Modify the CSS file (source has space after colon)
    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    // Wait for file watcher to pick up the change
    std::thread::sleep(Duration::from_millis(500));

    // Fetch the HTML again to get the new CSS URL (hash should change)
    let css_url_2 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("CSS URL should exist after change");

    // The CSS URL hash should have changed
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification"
    );

    // Fetch the new CSS content (minified)
    let css_2 = site.get(&css_url_2);
    css_2.assert_contains("font-weight:700");
}

/// Test CSS livereload in TUI mode using docs directory (mimics user workflow)
/// NOTE: Requires real PTY - ignored until we add portable-pty testing
#[test_log::test]
#[ignore = "TUI mode requires real terminal, needs PTY testing setup"]
fn test_css_livereload_tui_docs() {
    let site = TestSite::new("sample-site");

    // Ensure baseline CSS content
    const BASELINE_CSS: &str = r#"/* Test CSS with font URLs */
@font-face {
    font-family: 'TestFont';
    src: url('/fonts/test.woff2') format('woff2');
    font-weight: 400;
    font-style: normal;
}

body {
    font-family: 'TestFont', sans-serif;
}
"#;
    site.write_file("static/css/style.css", BASELINE_CSS);

    // Get initial CSS URL
    let css_url_1 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("Initial CSS URL should exist");

    // Verify initial CSS content (minified, so no space after colon)
    let css_1 = site.get(&css_url_1);
    css_1.assert_contains("font-weight:400");

    // Modify the CSS file (source has space after colon)
    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    // Wait for file watcher to pick up the change
    std::thread::sleep(Duration::from_millis(500));

    // Fetch the HTML again to get the new CSS URL (hash should change)
    let css_url_2 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("CSS URL should exist after change");

    // The CSS URL hash should have changed
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification"
    );

    // Fetch the new CSS content (minified)
    let css_2 = site.get(&css_url_2);
    css_2.assert_contains("font-weight:700");
}
