use super::*;

pub fn adding_page_updates_section_pages_list() {
    let site = TestSite::new("sample-site");

    // Helper function to extract and log page titles consistently
    let extract_page_titles = |html: &str, context: &str| -> Vec<String> {
        // (?s) enables DOTALL mode so .* matches across newlines (minification is disabled)
        let nav_re = regex::Regex::new(r#"(?s)<nav id="page-list">(.*?)</nav>"#).unwrap();
        if let Some(caps) = nav_re.captures(html) {
            let nav_html = &caps[1];
            let title_re = regex::Regex::new(r#">([^<]+)</a>"#).unwrap();
            let titles: Vec<String> = title_re
                .captures_iter(nav_html)
                .map(|c| c.get(1).unwrap().as_str().trim().to_string())
                .collect();
            tracing::debug!("{}: Found {} pages: {:?}", context, titles.len(), titles);
            titles
        } else {
            tracing::debug!("{}: No page-list nav found in HTML", context);
            Vec::new()
        }
    };

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

            // (?s) enables DOTALL mode so .* matches across newlines (minification is disabled)
            let nav_re = regex::Regex::new(r#"(?s)<nav id="page-list">(.*?)</nav>"#).unwrap();
            if nav_re.is_match(&html.body) {
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
