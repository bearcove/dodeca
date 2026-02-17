use super::*;
use hotmeal::{Document, NodeId, NodeKind, StrTendril};

fn find_nav_by_id(doc: &Document, node_id: NodeId, nav_id: &str) -> Option<NodeId> {
    let node = doc.get(node_id);
    if let NodeKind::Element(elem) = &node.kind
        && elem.tag.as_ref() == "nav"
        && elem
            .attrs
            .iter()
            .any(|(name, value)| name.local.as_ref() == "id" && value.as_ref() == nav_id)
    {
        return Some(node_id);
    }

    for child_id in doc.children(node_id) {
        if let Some(found) = find_nav_by_id(doc, child_id, nav_id) {
            return Some(found);
        }
    }

    None
}

fn collect_text(doc: &Document, node_id: NodeId, out: &mut String) {
    match &doc.get(node_id).kind {
        NodeKind::Text(text) => out.push_str(text.as_ref()),
        NodeKind::Element(_) | NodeKind::Document => {
            for child_id in doc.children(node_id) {
                collect_text(doc, child_id, out);
            }
        }
        NodeKind::Comment(_) => {}
    }
}

fn collect_link_titles(doc: &Document, node_id: NodeId, titles: &mut Vec<String>) {
    let node = doc.get(node_id);
    if let NodeKind::Element(elem) = &node.kind
        && elem.tag.as_ref() == "a"
    {
        let mut text = String::new();
        collect_text(doc, node_id, &mut text);
        let title = text.trim();
        if !title.is_empty() {
            titles.push(title.to_string());
        }
    }

    for child_id in doc.children(node_id) {
        collect_link_titles(doc, child_id, titles);
    }
}

fn nav_exists(html: &str, nav_id: &str) -> bool {
    let tendril = StrTendril::from(html);
    let doc = hotmeal::parse(&tendril);
    find_nav_by_id(&doc, doc.root, nav_id).is_some()
}

fn extract_nav_titles(html: &str, nav_id: &str, context: &str) -> Vec<String> {
    let tendril = StrTendril::from(html);
    let doc = hotmeal::parse(&tendril);

    let Some(nav_node) = find_nav_by_id(&doc, doc.root, nav_id) else {
        tracing::debug!("{}: No {} nav found in HTML", context, nav_id);
        return Vec::new();
    };

    let mut titles = Vec::new();
    collect_link_titles(&doc, nav_node, &mut titles);
    tracing::debug!("{}: Found {} pages: {:?}", context, titles.len(), titles);
    titles
}

fn extract_page_titles(html: &str, context: &str) -> Vec<String> {
    extract_nav_titles(html, "page-list", context)
}

pub fn adding_page_updates_section_pages_list() {
    let site = TestSite::new("sample-site");

    // First, do an initial request to make sure the site is responding
    tracing::debug!("Doing initial request to establish baseline");
    let initial_response = site.get("/guide/");
    initial_response.assert_ok();
    tracing::debug!("Site is responding with status {}", initial_response.status);

    // Check initial state
    let _initial_titles = extract_page_titles(&initial_response.body, "Initial state");

    tracing::debug!("Setting up section template with page list");
    site.write_file(
        "templates/section.html",
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="page-list">
    {% for page in section.pages %}
      <a href="{{ page.permalink }}">{{ page.title }}</a>
    {% endfor %}
  </nav>
</body>
</html>
"#,
    );

    site.wait_debounce();

    tracing::debug!("Verifying template is applied and section pages are generated");
    let html = site.wait_until(
        "template to be applied and page list to be generated",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            tracing::debug!("Template check response status: {}", html.status);

            if html.status != 200 {
                tracing::debug!("Non-200 status, retrying...");
                return None;
            }

            if nav_exists(&html.body, "page-list") {
                tracing::debug!("Found page-list nav, template successfully applied");
                Some(html)
            } else {
                tracing::debug!("Template not yet applied, page-list nav not found");
                None
            }
        },
    );

    extract_page_titles(&html.body, "After template applied");
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");

    tracing::debug!("Adding new page: new-topic.md");
    site.write_file(
        "content/guide/new-topic.md",
        r#"+++
title = "New Topic"
weight = 50
+++

# New Topic

This is a newly added page.
"#,
    );

    site.wait_debounce();

    tracing::debug!("Waiting for section pages list to update with new page");
    let updated_html = site.wait_until(
        "new page to appear in section pages list",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            tracing::debug!("Update check response status: {}", html.status);

            if html.status != 200 {
                tracing::debug!("Non-200 status during update check, retrying...");
                return None;
            }

            let current_titles = extract_page_titles(&html.body, "Update check");

            if current_titles.contains(&"New Topic".to_string()) {
                tracing::debug!("New Topic found in page list, update successful");
                Some(html)
            } else {
                tracing::debug!("New Topic not yet in page list, retrying...");
                None
            }
        },
    );

    tracing::debug!("Final verification: all pages should be present");
    updated_html.assert_contains("Getting Started");
    updated_html.assert_contains("Advanced");
    updated_html.assert_contains("New Topic");
}

pub fn adding_page_updates_via_get_section_macro() {
    let site = TestSite::new("sample-site");

    // First, do an initial request to make sure the site is responding
    tracing::debug!("Doing initial request to establish baseline");
    let initial_response = site.get("/guide/");
    initial_response.assert_ok();
    tracing::debug!("Site is responding with status {}", initial_response.status);

    tracing::debug!("Setting up macro template");
    site.write_file(
        "templates/macros.html",
        r#"{% macro render_section_pages(section_path) %}
    {% set sec = get_section(path=section_path) %}
    <ul class="section-pages">
    {% for page in sec.pages %}
        <li><a href="{{ page.permalink }}">{{ page.title }}</a></li>
    {% endfor %}
    </ul>
{% endmacro %}
"#,
    );

    tracing::debug!("Setting up section template with macro import");
    site.write_file(
        "templates/section.html",
        r#"{% import "macros.html" as macros %}
<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="macro-page-list">
    {{ macros::render_section_pages(section_path=section.path) }}
  </nav>
</body>
</html>
"#,
    );

    site.wait_debounce();

    tracing::debug!("Waiting for templates to be applied and macro to render");
    let html = site.wait_until(
        "get_section macro to show section-pages",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            tracing::debug!("Macro check response status: {}", html.status);

            if html.status != 200 {
                tracing::debug!("Non-200 status, retrying...");
                return None;
            }

            if html.body.contains("section-pages") {
                tracing::debug!("Found section-pages class in HTML");
                Some(html)
            } else {
                tracing::debug!("section-pages class not found yet, retrying...");
                None
            }
        },
    );

    html.assert_ok();
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");
    html.assert_contains("section-pages");

    tracing::debug!("Adding new page to test macro update");
    site.write_file(
        "content/guide/macro-test-page.md",
        r#"+++
title = "Macro Test Page"
weight = 100
+++

# Macro Test Page

Testing get_section in macros.
"#,
    );

    site.wait_debounce();

    tracing::debug!("Waiting for macro to include new page");
    let updated_html = site.wait_until(
        "get_section macro to include new macro test page",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            tracing::debug!("Macro update check response status: {}", html.status);

            if html.status != 200 {
                tracing::debug!("Non-200 status during update check, retrying...");
                return None;
            }

            if html.body.contains("Macro Test Page") {
                tracing::debug!("Found Macro Test Page in updated HTML");
                Some(html)
            } else {
                tracing::debug!("Macro Test Page not found yet, retrying...");
                None
            }
        },
    );

    tracing::debug!("Final verification: macro test page should be present");
    updated_html.assert_ok();
    updated_html.assert_contains("Macro Test Page");
}

pub fn removing_page_updates_via_get_section_macro() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "templates/macros.html",
        r#"{% macro render_section_pages(section_path) %}
    {% set sec = get_section(path=section_path) %}
    <ul class="section-pages">
    {% for page in sec.pages %}
        <li><a href="{{ page.permalink }}">{{ page.title }}</a></li>
    {% endfor %}
    </ul>
{% endmacro %}
"#,
    );

    site.write_file(
        "templates/section.html",
        r#"{% import "macros.html" as macros %}
<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="macro-page-list">
    {{ macros::render_section_pages(section_path=section.path) }}
  </nav>
</body>
</html>
"#,
    );

    site.wait_debounce();

    let baseline = site.wait_until(
        "baseline macro section pages to be visible",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            if html.status != 200 || !html.body.contains("macro-page-list") {
                return None;
            }
            if html.body.contains("Getting Started") && html.body.contains("Advanced") {
                Some(html)
            } else {
                None
            }
        },
    );
    baseline.assert_contains("Getting Started");
    baseline.assert_contains("Advanced");

    site.delete_file("content/guide/advanced.md");
    site.wait_debounce();

    let _deleted_page = site.wait_until(
        "deleted page route to return 404 (macro test)",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/advanced/");
            if html.status == 404 { Some(html) } else { None }
        },
    );

    let updated = site.wait_until(
        "macro get_section pages list to drop deleted page",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            if html.status != 200 {
                return None;
            }
            let titles = extract_nav_titles(&html.body, "macro-page-list", "Macro after deletion");
            if titles.iter().any(|t| t == "Advanced") {
                None
            } else if !titles.is_empty() {
                Some(html)
            } else {
                None
            }
        },
    );

    let titles = extract_nav_titles(
        &updated.body,
        "macro-page-list",
        "Macro final after deletion",
    );
    assert!(
        titles.iter().any(|t| t == "Getting Started"),
        "Expected Getting Started in macro section list, got {:?}",
        titles
    );
    assert!(
        !titles.iter().any(|t| t == "Advanced"),
        "Expected Advanced to be removed from macro section list, got {:?}",
        titles
    );
}

pub fn removing_sibling_page_updates_page_section_pages_list() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "templates/page.html",
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ page.title }}</title>
</head>
<body>
  <h1>{{ page.title }}</h1>
  <nav id="sibling-page-list">
    {% for p in section.pages %}
      <a href="{{ p.permalink }}">{{ p.title }}</a>
    {% endfor %}
  </nav>
  {{ page.content | safe }}
</body>
</html>
"#,
    );

    site.wait_debounce();

    let baseline = site.wait_until(
        "page template with sibling list to render",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/getting-started/");
            if html.status != 200 {
                return None;
            }
            let titles = extract_nav_titles(&html.body, "sibling-page-list", "Page baseline");
            if titles.iter().any(|t| t == "Getting Started")
                && titles.iter().any(|t| t == "Advanced")
            {
                Some(html)
            } else {
                None
            }
        },
    );
    baseline.assert_contains("Getting Started");

    site.delete_file("content/guide/advanced.md");
    site.wait_debounce();

    let _deleted_page = site.wait_until(
        "deleted sibling route to return 404",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/advanced/");
            if html.status == 404 { Some(html) } else { None }
        },
    );

    let updated = site.wait_until(
        "sibling list on page to drop deleted page",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/getting-started/");
            if html.status != 200 {
                return None;
            }
            let titles = extract_nav_titles(&html.body, "sibling-page-list", "Page after deletion");
            if titles.iter().any(|t| t == "Advanced") {
                None
            } else {
                Some(html)
            }
        },
    );

    let titles = extract_nav_titles(
        &updated.body,
        "sibling-page-list",
        "Page final after deletion",
    );
    assert!(
        titles.iter().any(|t| t == "Getting Started"),
        "Expected Getting Started in sibling list, got {:?}",
        titles
    );
    assert!(
        !titles.iter().any(|t| t == "Advanced"),
        "Expected Advanced to be removed from sibling list, got {:?}",
        titles
    );
}

pub fn removing_page_updates_section_pages_list() {
    let site = TestSite::new("sample-site");

    site.write_file(
        "templates/section.html",
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="page-list">
    {% for page in section.pages %}
      <a href="{{ page.permalink }}">{{ page.title }}</a>
    {% endfor %}
  </nav>
</body>
</html>
"#,
    );

    site.wait_debounce();

    // Ensure baseline includes both sample pages.
    let initial_html = site.wait_until(
        "baseline section pages to be visible",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            if html.status == 200 && nav_exists(&html.body, "page-list") {
                Some(html)
            } else {
                None
            }
        },
    );
    initial_html.assert_contains("Getting Started");
    initial_html.assert_contains("Advanced");

    site.delete_file("content/guide/advanced.md");
    site.wait_debounce();

    let _deleted_page = site.wait_until(
        "deleted page route to return 404",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/advanced/");
            if html.status == 404 { Some(html) } else { None }
        },
    );

    let updated_html = site.wait_until(
        "deleted page to disappear from section pages list",
        Duration::from_secs(2),
        || {
            let html = site.get("/guide/");
            if html.status != 200 {
                return None;
            }
            let titles = extract_page_titles(&html.body, "After deletion");
            if titles.iter().any(|t| t == "Advanced") {
                None
            } else {
                Some(html)
            }
        },
    );

    let titles = extract_page_titles(&updated_html.body, "Final after deletion");
    assert!(
        titles.iter().any(|t| t == "Getting Started"),
        "Expected Getting Started in section.pages, got {:?}",
        titles
    );
    assert!(
        !titles.iter().any(|t| t == "Advanced"),
        "Expected Advanced to be removed from section.pages, got {:?}",
        titles
    );
}

#[cfg(test)]
mod unit_tests {
    use super::extract_page_titles;

    #[test]
    fn extract_page_titles_from_nav_links() {
        let html = r#"<!doctype html>
<html>
  <body>
    <nav id="page-list">
      <a href="/one">One</a>
      <a href="/two"><span>Two</span></a>
    </nav>
  </body>
</html>
"#;
        let titles = extract_page_titles(html, "test");
        assert_eq!(titles, vec!["One".to_string(), "Two".to_string()]);
        assert!(extract_page_titles("<html></html>", "test").is_empty());
    }
}
