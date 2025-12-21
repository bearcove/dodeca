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

const RENDER_ERROR_MARKER: &str = "<!-- DODECA_RENDER_ERROR -->";
