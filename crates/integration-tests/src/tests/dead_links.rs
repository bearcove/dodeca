use super::*;

pub fn dead_links_marked_in_html() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "content/dead-link-test.md",
        r#"---
title: Dead Link Test
---

Check out [this broken link](/nonexistent-page/).
"#,
    );

    site.wait_debounce();

    let html = site.get("/dead-link-test/");
    html.assert_ok();
    html.assert_contains("data-dead");
}

pub fn valid_links_not_marked_dead() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();

    if html.text().contains("/guide/") {
        assert!(
            !html.text().contains(r#"href="/guide/" data-dead"#),
            "Valid links should not be marked as dead"
        );
    }
}
