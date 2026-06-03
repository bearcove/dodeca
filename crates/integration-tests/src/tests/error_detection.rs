use super::*;

pub async fn template_syntax_error_shows_error_page() {
    let site = TestSite::new("sample-site");

    let html = site.get("/").await;
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");

    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let html = site.get("/").await;
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
    html.assert_contains("Template Error");
}

pub async fn template_error_recovery_removes_error_page() {
    let site = TestSite::new("sample-site");

    let original = site.read_file("templates/index.html");

    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let html = site.get("/").await;
    html.assert_contains(RENDER_ERROR_MARKER);

    site.write_file("templates/index.html", &original);

    tokio::time::sleep(Duration::from_millis(500)).await;

    let html = site.get("/").await;
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");
}

pub async fn missing_template_shows_error_page() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/").await;
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);

    site.delete_file("templates/section.html");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let html = site.get("/guide/").await;
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
}

pub async fn type_error_shows_ariadne_formatted_source() {
    let site = TestSite::new("sample-site");

    // Introduce a type error by accessing a field on a potentially-none value
    site.modify_file("templates/section.html", |content| {
        // Add a line that will cause a TypeError when extra is none
        content.replace(
            "{{ section.title }}",
            "{{ section.extra.nonexistent_field }}",
        )
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    let html = site.get("/guide/").await;
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);

    // Verify the HTML error page shows source context with line numbers
    // The error page renders styled HTML with source-context, line-number, and error-marker classes
    let body = &html.body;
    let has_source_context = body.contains("source-context");
    let has_line_numbers = body.contains("line-number");
    let has_error_marker = body.contains("error-marker");

    assert!(
        has_source_context && has_line_numbers && has_error_marker,
        "Expected HTML error page with source context.\nActual body contains: {}",
        if body.len() > 500 {
            format!("{}...", &body[..500])
        } else {
            body.clone()
        }
    );
}

pub async fn frontmatter_parse_error_shows_error_page_and_recovers() {
    let site = TestSite::new("sample-site");
    let original = site.read_file("content/guide/getting-started.md");

    site.modify_file("content/guide/getting-started.md", |content| {
        content.replace("title = \"Getting Started\"", "title: Getting Started")
    });

    site.wait_debounce().await;

    let html = site.get("/").await;
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
    html.assert_contains("Failed to parse 1 file");
    html.assert_contains("guide/getting-started.md");

    site.write_file("content/guide/getting-started.md", &original);
    site.wait_debounce().await;

    let html = site.get("/").await;
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");
}

const RENDER_ERROR_MARKER: &str = "data-dodeca-error";
