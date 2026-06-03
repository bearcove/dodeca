use super::*;

pub async fn dead_links_marked_in_html() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "content/dead-link-test.md",
        r#"---
title: Dead Link Test
---

Check out [this broken link](/nonexistent-page/).
"#,
    );

    site.wait_debounce().await;

    let html = site.get("/dead-link-test/").await;
    html.assert_ok();
    html.assert_contains("data-dead");
}

pub async fn valid_links_not_marked_dead() {
    let site = TestSite::new("sample-site");

    let html = site.get("/").await;
    html.assert_ok();

    if html.text().contains("/guide/") {
        assert!(
            !html.text().contains(r#"href="/guide/" data-dead"#),
            "Valid links should not be marked as dead"
        );
    }
}
