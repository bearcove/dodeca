use super::*;

/// Test that @/ links in list items are resolved correctly
pub fn at_links_in_list_items_resolved() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "content/link-test.md",
        r#"---
title: Link Test
---

## Links in list items

- [Link to guide](@/guide/_index.md)
- [`code link`](@/guide/_index.md) — with description
"#,
    );

    site.wait_debounce();

    let html = site.get("/link-test/");
    html.assert_ok();

    // The @/ should be resolved, not left in the href
    assert!(
        !html.text().contains("@/"),
        "@/ should be resolved in list item links, got: {}",
        html.text()
    );

    // Should resolve to the correct path
    assert!(
        html.text().contains(r#"href="/guide/""#),
        "Link should resolve to /guide/, got: {}",
        html.text()
    );
}

/// Test that @/ links in paragraphs are resolved correctly
pub fn at_links_in_paragraphs_resolved() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "content/para-link-test.md",
        r#"---
title: Paragraph Link Test
---

Check out [the guide](@/guide/_index.md) for more information.

See also [getting started](@/guide/_index.md#getting-started) for quick setup.
"#,
    );

    site.wait_debounce();

    let html = site.get("/para-link-test/");
    html.assert_ok();

    assert!(
        !html.text().contains("@/"),
        "@/ should be resolved in paragraph links"
    );

    assert!(
        html.text().contains(r#"href="/guide/""#),
        "Link should resolve to /guide/"
    );

    assert!(
        html.text().contains(r#"href="/guide/#getting-started""#),
        "Link with fragment should resolve correctly"
    );
}

/// Test that relative .md links are resolved correctly
pub fn relative_md_links_resolved() {
    let site = TestSite::new("sample-site");

    // Create a page that links to a sibling
    site.write_file(
        "content/guide/page-a.md",
        r#"---
title: Page A
---

See [Page B](page-b.md) for more.
"#,
    );

    site.write_file(
        "content/guide/page-b.md",
        r#"---
title: Page B
---

This is page B.
"#,
    );

    site.wait_debounce();

    let html = site.get("/guide/page-a/");
    html.assert_ok();

    // The .md should be stripped and resolved to proper route
    assert!(
        !html.text().contains(r#"href="page-b.md""#),
        "Raw .md link should not appear in href"
    );

    assert!(
        html.text().contains(r#"href="/guide/page-b/""#),
        "Relative .md link should resolve to /guide/page-b/"
    );
}
