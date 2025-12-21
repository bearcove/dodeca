use super::*;

pub fn adding_page_updates_section_pages_list() {
    let site = TestSite::new("sample-site");

    tracing::info!("Setting up section template with page list");
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

    tracing::info!("Checking initial section pages");

    let html = site.wait_until(
        "initial section pages list to be generated",
        Duration::from_secs(5),
        || {
            let html = site.get("/guide/");
            html.assert_ok();

            let nav_re = regex::Regex::new(r#"<nav id="page-list">(.*?)</nav>"#).unwrap();
            if nav_re.is_match(&html.body) {
                Some(html)
            } else {
                None
            }
        },
    );

    // Extract page titles from the navigation
    let nav_re = regex::Regex::new(r#"<nav id="page-list">(.*?)</nav>"#).unwrap();
    if let Some(caps) = nav_re.captures(&html.body) {
        let nav_html = &caps[1];
        let title_re = regex::Regex::new(r#">([^<]+)</a>"#).unwrap();
        let titles: Vec<&str> = title_re
            .captures_iter(nav_html)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        tracing::info!("Found {} pages in section: {:?}", titles.len(), titles);
    } else {
        tracing::warn!("Could not find page-list nav in HTML");
    }

    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");

    tracing::info!("Adding new page: new-topic.md");
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

    tracing::info!("Waiting for section.pages to update with new page");
    site.wait_until(
        "section pages list to include new topic",
        Duration::from_secs(5),
        || {
            tracing::debug!("Getting...");
            let html = site.get("/guide/");

            // Show what we found
            tracing::debug!("Applying RE...");
            let nav_re = regex::Regex::new(r#"<nav id="page-list">(.*?)</nav>"#).unwrap();
            if let Some(caps) = nav_re.captures(&html.body) {
                let nav_html = &caps[1];
                let title_re = regex::Regex::new(r#">([^<]+)</a>"#).unwrap();
                let titles: Vec<&str> = title_re
                    .captures_iter(nav_html)
                    .map(|c| c.get(1).unwrap().as_str())
                    .collect();
                tracing::debug!("Poll: Found {} pages: {:?}", titles.len(), titles);
            } else {
                tracing::error!(
                    "Poll: Did not find nav section. Entire markup: {}",
                    html.body
                );
                panic!("Markup did not have page-list");
            }

            if html.body.contains("New Topic") {
                Some(html)
            } else {
                None
            }
        },
    );

    tracing::info!("Final check: all pages should be present");
    let html = site.get("/guide/");
    html.assert_ok();

    // Show final state
    let nav_re = regex::Regex::new(r#"<nav id="page-list">(.*?)</nav>"#).unwrap();
    if let Some(caps) = nav_re.captures(&html.body) {
        let nav_html = &caps[1];
        let title_re = regex::Regex::new(r#">([^<]+)</a>"#).unwrap();
        let titles: Vec<&str> = title_re
            .captures_iter(nav_html)
            .map(|c| c.get(1).unwrap().as_str())
            .collect();
        tracing::info!("Final state: {} pages: {:?}", titles.len(), titles);
    }

    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");
    html.assert_contains("New Topic");
}

pub fn adding_page_updates_via_get_section_macro() {
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

    site.wait_until(
        "get_section macro to show section-pages",
        Duration::from_secs(5),
        || {
            let html = site.get("/guide/");
            if html.body.contains("section-pages") {
                Some(html)
            } else {
                None
            }
        },
    );

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Getting Started");
    html.assert_contains("Advanced");
    html.assert_contains("section-pages");

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

    site.wait_until(
        "get_section macro to include new macro test page",
        Duration::from_secs(5),
        || {
            let html = site.get("/guide/");
            if html.body.contains("Macro Test Page") {
                Some(html)
            } else {
                None
            }
        },
    );

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains("Macro Test Page");
}
