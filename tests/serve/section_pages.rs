//! Tests for section.pages cache invalidation
//!
//! Verifies that when a new page is added to a section, templates that iterate
//! through section.pages are properly updated.

use crate::harness::TestSite;
use std::time::Duration;

/// Test that adding a new page to a section updates the section.pages list
/// in templates that iterate through it.
#[test_log::test]
fn adding_page_updates_section_pages_list() {
    let site = TestSite::new("sample-site");

    // First, modify the section template to list pages
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

    // Wait for the template change to be picked up
    site.wait_debounce();

    // Verify the guide section shows existing pages
    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");

    // Now add a new page to the guide section
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

    // Wait for the file watcher to pick it up and rebuild
    site.wait_debounce();

    // The section page should now list the new page
    site.wait_until(Duration::from_secs(5), || {
        let html = site.get("/guide/");
        if html.body.contains("New Topic") {
            Some(html)
        } else {
            None
        }
    });

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");
    html.assert_contains("New Topic"); // The new page should appear
}

/// Test that adding a new page updates when accessed via get_section() in a macro
/// This mimics the facet docs pattern where macros use get_section(path=...)
#[test_log::test]
fn adding_page_updates_via_get_section_macro() {
    let site = TestSite::new("sample-site");

    // Create a macros template with get_section pattern (like facet docs)
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

    // Modify section template to import and use the macro
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

    // Verify initial state
    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");
    html.assert_contains("section-pages"); // Macro rendered

    // Add a new page
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

    // The new page should appear via the macro's get_section call
    site.wait_until(Duration::from_secs(5), || {
        let html = site.get("/guide/");
        if html.body.contains("Macro Test Page") {
            Some(html)
        } else {
            None
        }
    });

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Macro Test Page");
}
