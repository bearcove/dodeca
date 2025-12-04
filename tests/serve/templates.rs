//! Template rendering tests

use crate::harness::TestSite;

#[test_log::test]
fn template_renders_content() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();

    // Template should wrap content in proper HTML structure
    // Note: minified HTML uses lowercase doctype
    html.assert_contains("<!doctype html>");
    html.assert_contains("<body>");
}

#[test_log::test]
fn template_includes_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // Template should include CSS links
    assert!(
        html.text().contains("stylesheet") || html.text().contains(".css"),
        "Template should include CSS references"
    );
}

#[test_log::test]
fn template_metadata_used() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // Frontmatter title should be used in <title> tag
    html.assert_contains("<title>Home</title>");
}

#[test_log::test]
fn different_templates_for_different_pages() {
    let site = TestSite::new("sample-site");

    // Both pages should render successfully
    let index = site.get("/");
    let guide = site.get("/guide/");

    index.assert_ok();
    guide.assert_ok();

    // Both should have proper HTML structure (minified)
    index.assert_contains("<!doctype html>");
    guide.assert_contains("<!doctype html>");
}

#[test_log::test]
fn extra_frontmatter_accessible_in_templates() {
    let site = TestSite::new("sample-site");

    // Add [extra] to the guide section frontmatter
    site.write_file(
        "content/guide/_index.md",
        r#"+++
title = "Guide"
[extra]
sidebar = true
icon = "book"
custom_value = 42
+++

# Guide

This is the guide section.
"#,
    );

    // Update the section template to display extra fields
    site.write_file(
        "templates/section.html",
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {% if section.extra.sidebar %}
  <div class="has-sidebar" data-icon="{{ section.extra.icon }}" data-custom="{{ section.extra.custom_value }}">Sidebar enabled</div>
  {% endif %}
  {{ section.content | safe }}
</body>
</html>
"#,
    );

    // Wait for livereload
    std::thread::sleep(std::time::Duration::from_millis(500));

    let html = site.get("/guide/");
    html.assert_ok();

    // Check that extra fields are rendered
    // Note: HTML minifier removes quotes from attribute values
    html.assert_contains("has-sidebar");
    html.assert_contains("data-icon=book");
    html.assert_contains("data-custom=42");
    html.assert_contains("Sidebar enabled");
}

#[test_log::test]
fn page_extra_frontmatter_accessible_in_templates() {
    let site = TestSite::new("sample-site");

    // Add [extra] to a page frontmatter
    site.write_file(
        "content/guide/getting-started.md",
        r#"+++
title = "Getting Started"
[extra]
difficulty = "beginner"
reading_time = 5
+++

# Getting Started

This is the getting started guide.
"#,
    );

    // Update the page template to display extra fields
    site.write_file(
        "templates/page.html",
        r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ page.title }}</title>
</head>
<body>
  <h1>{{ page.title }}</h1>
  <div class="meta" data-difficulty="{{ page.extra.difficulty }}" data-time="{{ page.extra.reading_time }}">
    Difficulty: {{ page.extra.difficulty }}, Reading time: {{ page.extra.reading_time }} min
  </div>
  {{ page.content | safe }}
</body>
</html>
"#,
    );

    // Wait for livereload
    std::thread::sleep(std::time::Duration::from_millis(500));

    let html = site.get("/guide/getting-started/");
    html.assert_ok();

    // Check that extra fields are rendered
    html.assert_contains("data-difficulty=beginner");
    html.assert_contains("data-time=5");
    html.assert_contains("Difficulty: beginner");
    html.assert_contains("Reading time: 5 min");
}

#[test_log::test]
fn code_blocks_have_copy_button_script() {
    let site = TestSite::new("sample-site");

    // Add a page with a code block
    site.write_file(
        "content/guide/code-example.md",
        r#"+++
title = "Code Example"
+++

# Code Example

Here's some code:

```rust
fn main() {
    println!("Hello, world!");
}
```
"#,
    );

    // Wait for livereload
    std::thread::sleep(std::time::Duration::from_millis(500));

    let html = site.get("/guide/code-example/");
    html.assert_ok();

    // Check that the copy button script and styles are injected
    html.assert_contains(".copy-btn");
    html.assert_contains("navigator.clipboard.writeText");
    // Check that the code block is rendered
    html.assert_contains("<code");
    html.assert_contains("Hello, world!");
}
