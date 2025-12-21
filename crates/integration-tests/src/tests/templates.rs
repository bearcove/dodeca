use super::*;

pub fn template_renders_content() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("<!DOCTYPE html>");
    html.assert_contains("<body>");
}

pub fn template_includes_css() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    assert!(
        html.text().contains("stylesheet") || html.text().contains(".css"),
        "Template should include CSS references"
    );
}

pub fn template_metadata_used() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    html.assert_contains("<title>Home</title>");
}

pub fn different_templates_for_different_pages() {
    let site = TestSite::new("sample-site");
    let index = site.get("/");
    let guide = site.get("/guide/");
    index.assert_ok();
    guide.assert_ok();
    index.assert_contains("<!DOCTYPE html>");
    guide.assert_contains("<!DOCTYPE html>");
}

pub fn extra_frontmatter_accessible_in_templates() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
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
            ),
            (
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
            ),
        ],
    );

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("has-sidebar");
    html.assert_contains("data-icon");
    html.assert_contains("book");
    html.assert_contains("data-custom");
    html.assert_contains("42");
    html.assert_contains("Sidebar enabled");
}

pub fn page_extra_frontmatter_accessible_in_templates() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
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
            ),
            (
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
            ),
        ],
    );

    let html = site.get("/guide/getting-started/");
    html.assert_ok();
    html.assert_contains("data-difficulty");
    html.assert_contains("beginner");
    html.assert_contains("data-time");
    html.assert_contains("Difficulty: beginner");
    html.assert_contains("Reading time: 5 min");
}

pub fn code_blocks_have_copy_button_script() {
    let site = TestSite::with_files(
        "sample-site",
        &[(
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
        )],
    );

    let html = site.get("/guide/code-example/");
    html.assert_ok();
    html.assert_contains(".copy-btn");
    html.assert_contains("navigator.clipboard.writeText");
    html.assert_contains("<code");
    html.assert_contains("Hello, world!");
}
