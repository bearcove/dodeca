use super::*;
use facet_html_dom::{FlowContent, Html, PhrasingContent};

/// Extract all href values from an HTML document
fn extract_hrefs(doc: &Html) -> Vec<String> {
    let mut hrefs = Vec::new();
    if let Some(body) = &doc.body {
        collect_hrefs_from_flow(&body.children, &mut hrefs);
    }
    hrefs
}

fn collect_hrefs_from_flow(children: &[FlowContent], hrefs: &mut Vec<String>) {
    for child in children {
        match child {
            FlowContent::A(a) => {
                if let Some(href) = &a.href {
                    hrefs.push(href.clone());
                }
                collect_hrefs_from_phrasing(&a.children, hrefs);
            }
            FlowContent::P(p) => {
                collect_hrefs_from_phrasing(&p.children, hrefs);
            }
            FlowContent::Div(div) => {
                collect_hrefs_from_flow(&div.children, hrefs);
            }
            FlowContent::Ul(ul) => {
                for li in &ul.li {
                    collect_hrefs_from_flow(&li.children, hrefs);
                }
            }
            FlowContent::Ol(ol) => {
                for li in &ol.li {
                    collect_hrefs_from_flow(&li.children, hrefs);
                }
            }
            FlowContent::Article(article) => {
                collect_hrefs_from_flow(&article.children, hrefs);
            }
            FlowContent::Section(section) => {
                collect_hrefs_from_flow(&section.children, hrefs);
            }
            FlowContent::Main(main) => {
                collect_hrefs_from_flow(&main.children, hrefs);
            }
            FlowContent::Nav(nav) => {
                collect_hrefs_from_flow(&nav.children, hrefs);
            }
            FlowContent::Header(header) => {
                collect_hrefs_from_flow(&header.children, hrefs);
            }
            FlowContent::Footer(footer) => {
                collect_hrefs_from_flow(&footer.children, hrefs);
            }
            _ => {}
        }
    }
}

fn collect_hrefs_from_phrasing(children: &[PhrasingContent], hrefs: &mut Vec<String>) {
    for child in children {
        match child {
            PhrasingContent::A(a) => {
                if let Some(href) = &a.href {
                    hrefs.push(href.clone());
                }
                collect_hrefs_from_phrasing(&a.children, hrefs);
            }
            PhrasingContent::Span(span) => {
                collect_hrefs_from_phrasing(&span.children, hrefs);
            }
            PhrasingContent::Em(em) => {
                collect_hrefs_from_phrasing(&em.children, hrefs);
            }
            PhrasingContent::Strong(strong) => {
                collect_hrefs_from_phrasing(&strong.children, hrefs);
            }
            PhrasingContent::Code(code) => {
                collect_hrefs_from_phrasing(&code.children, hrefs);
            }
            _ => {}
        }
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

    let doc: Html = facet_html::from_str(html.text()).expect("Failed to parse HTML");
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

    let doc: Html = facet_html::from_str(html.text()).expect("Failed to parse HTML");
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

    let doc: Html = facet_html::from_str(html.text()).expect("Failed to parse HTML");
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
