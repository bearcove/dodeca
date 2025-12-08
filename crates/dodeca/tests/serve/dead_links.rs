//! Dead link detection tests

use crate::harness::TestSite;

#[test_log::test]
fn dead_links_marked_in_html() {
    let site = TestSite::new("sample-site");

    // Create a page with a dead link
    site.write_file(
        "content/dead-link-test.md",
        r#"---
title: Dead Link Test
---

Check out [this broken link](/nonexistent-page/).
"#,
    );

    // Wait for rebuild
    site.wait_debounce();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let html = site.get("/dead-link-test/");
    html.assert_ok();

    // Dead links should be marked with data-dead attribute
    html.assert_contains("data-dead");
}

#[test_log::test]
fn valid_links_not_marked_dead() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();

    // Links to existing pages should not have data-dead
    // The guide link should work
    if html.text().contains("/guide/") {
        assert!(
            !html.text().contains(r#"href="/guide/" data-dead"#),
            "Valid links should not be marked as dead"
        );
    }
}
