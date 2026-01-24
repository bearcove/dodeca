use super::*;

pub fn template_syntax_error_shows_error_page() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");

    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    std::thread::sleep(Duration::from_millis(500));

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
    html.assert_contains("Template Error");
}

pub fn template_error_recovery_removes_error_page() {
    let site = TestSite::new("sample-site");

    let original = site.read_file("templates/index.html");

    site.modify_file("templates/index.html", |content| {
        content.replace("{{ section.title }}", "{{ section.title")
    });

    std::thread::sleep(Duration::from_millis(500));

    let html = site.get("/");
    html.assert_contains(RENDER_ERROR_MARKER);

    site.write_file("templates/index.html", &original);

    std::thread::sleep(Duration::from_millis(500));

    let html = site.get("/");
    html.assert_not_contains(RENDER_ERROR_MARKER);
    html.assert_contains("<!DOCTYPE html>");
}

pub fn missing_template_shows_error_page() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_not_contains(RENDER_ERROR_MARKER);

    site.delete_file("templates/section.html");

    std::thread::sleep(Duration::from_millis(500));

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);
}

pub fn type_error_shows_ariadne_formatted_source() {
    let site = TestSite::new("sample-site");

    // Introduce a type error by accessing a field on a potentially-none value
    site.modify_file("templates/section.html", |content| {
        // Add a line that will cause a TypeError when extra is none
        content.replace(
            "{{ section.title }}",
            "{{ section.extra.nonexistent_field }}",
        )
    });

    std::thread::sleep(Duration::from_millis(500));

    let html = site.get("/guide/");
    html.assert_ok();
    html.assert_contains(RENDER_ERROR_MARKER);

    // Verify ariadne formatting is present in the error output
    // Ariadne uses box-drawing characters like │ (U+2502) and ─ (U+2500)
    // and shows source context with line numbers
    let body = &html.body;
    let has_ariadne_chars = body.contains('│') || body.contains("───");
    let has_line_indicator = body.contains(":1:") || body.contains(":2:");

    assert!(
        has_ariadne_chars || has_line_indicator,
        "Expected ariadne-formatted error with source context.\nActual body contains: {}",
        if body.len() > 500 {
            format!("{}...", &body[..500])
        } else {
            body.clone()
        }
    );
}

const RENDER_ERROR_MARKER: &str = "data-dodeca-error";
