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
  {% if page.extra and page.extra.difficulty %}
  <div class="meta" data-difficulty="{{ page.extra.difficulty }}" data-time="{{ page.extra.reading_time }}">
    Difficulty: {{ page.extra.difficulty }}, Reading time: {{ page.extra.reading_time }} min
  </div>
  {% endif %}
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

/// Test that syntax-highlighted code blocks preserve newlines
pub fn code_blocks_preserve_newlines() {
    let site = TestSite::with_files(
        "sample-site",
        &[(
            "content/guide/code-newlines.md",
            r#"+++
title = "Code Newlines"
+++

# Multi-line Code

```rust
fn greet(name: &str) {
    println!("Hello, {}!", name);
    println!("Welcome!");
}

fn main() {
    greet("World");
}
```
"#,
        )],
    );

    let html = site.get("/guide/code-newlines/");
    html.assert_ok();

    let full_body = html.text();

    // Extract just the <body> section for debugging
    let body_start = full_body.find("<body>").unwrap_or(0);
    let body = &full_body[body_start..];

    // Find the <code> block content
    assert!(
        body.contains("<code"),
        "Should have a <code> element. Body section:\n{}",
        &body[..body.len().min(3000)]
    );

    // Extract the code block for analysis
    let code_start = body.find("<code").unwrap_or(0);
    let code_end = body[code_start..]
        .find("</code>")
        .map(|i| code_start + i + 7)
        .unwrap_or(body.len());
    let code_block = &body[code_start..code_end];

    // The code should contain our function names (may be split by highlighting tags)
    // "greet" and "println" should both be present
    assert!(
        code_block.contains("greet") && code_block.contains("println"),
        "Code content should be present. Code block:\n{}",
        code_block
    );

    // Check for actual newline preservation.
    // The code has 8 lines (including blank line between functions).
    // If newlines are preserved, we should see them in the HTML.
    // If not, all code will be on a single line.
    let has_newlines = code_block.contains('\n');

    assert!(
        has_newlines,
        "Code blocks should preserve newlines between lines of code.\n\
         The code block appears to have all content on a single line.\n\
         Code block:\n{}",
        code_block
    );
}
