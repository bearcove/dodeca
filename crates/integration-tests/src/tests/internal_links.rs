use super::*;
use hotmeal::{Document, NodeId, NodeKind, StrTendril};

/// Extract all href values from an HTML document
fn extract_hrefs(doc: &Document) -> Vec<String> {
    let mut hrefs = Vec::new();
    if let Some(body) = doc.body() {
        collect_hrefs(doc, body, &mut hrefs);
    }
    hrefs
}

fn collect_hrefs(doc: &Document, node_id: NodeId, hrefs: &mut Vec<String>) {
    let node = doc.get(node_id);

    if let NodeKind::Element(elem) = &node.kind {
        // Check if this is an <a> tag with an href
        if elem.tag.as_ref() == "a" {
            for (name, value) in &elem.attrs {
                if name.local.as_ref() == "href" {
                    hrefs.push(value.as_ref().to_string());
                }
            }
        }
    }

    // Recurse into children
    for child_id in doc.children(node_id) {
        collect_hrefs(doc, child_id, hrefs);
    }
}

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

    let tendril = StrTendril::from(html.text());
    let doc = hotmeal::parse(&tendril);
    let hrefs = extract_hrefs(&doc);

    // Check that no links have @/ in their href (should be resolved)
    for href in &hrefs {
        assert!(
            !href.starts_with("@/"),
            "@/ should be resolved in list item links, found href={:?}",
            href
        );
    }

    // Should have a link to /guide/
    assert!(
        hrefs.iter().any(|h| h == "/guide/"),
        "Should have links to /guide/, found links: {:?}",
        hrefs
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

    let tendril = StrTendril::from(html.text());
    let doc = hotmeal::parse(&tendril);
    let hrefs = extract_hrefs(&doc);

    // Check no @/ links remain
    for href in &hrefs {
        assert!(
            !href.starts_with("@/"),
            "@/ should be resolved in paragraph links, found href={:?}",
            href
        );
    }

    // Should have a link to /guide/
    assert!(
        hrefs.iter().any(|h| h == "/guide/"),
        "Should have link to /guide/"
    );

    // Should have a link with fragment
    assert!(
        hrefs.iter().any(|h| h == "/guide/#getting-started"),
        "Should have link to /guide/#getting-started, found: {:?}",
        hrefs
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

    let tendril = StrTendril::from(html.text());
    let doc = hotmeal::parse(&tendril);
    let hrefs = extract_hrefs(&doc);

    // The .md should be stripped and resolved to proper route
    assert!(
        !hrefs.iter().any(|h| h == "page-b.md"),
        "Raw .md link should not appear in href"
    );

    // Should resolve to /guide/page-b/
    assert!(
        hrefs.iter().any(|h| h == "/guide/page-b/"),
        "Relative .md link should resolve to /guide/page-b/, found: {:?}",
        hrefs
    );
}
