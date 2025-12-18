//! SASS/SCSS compilation tests

use crate::harness::{InlineSite, TestSite};
use std::fs;
use std::time::Duration;

/// Test that a site without any SCSS files builds successfully
#[tokio::test]
async fn no_scss_builds_successfully() {
    let site = TestSite::new("no-scss-site");

    // Site should load without errors
    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("Welcome");

    // No main.css link should exist in output (template doesn't reference it)
    assert!(
        html.css_link("/main.*.css").is_none(),
        "No CSS should be generated when SCSS is absent"
    );
}

/// Test that SCSS files without main.scss entry point are gracefully skipped
#[tokio::test]
async fn scss_without_main_entry_point_skipped() {
    // Create a site with SCSS files but no main.scss
    let site = InlineSite::new(&[("_index.md", "+++\ntitle = \"Home\"\n+++\n\n# Hello")]);

    // Remove main.scss and create only a partial file
    fs::remove_file(site.fixture_dir.join("sass/main.scss")).expect("remove main.scss");
    fs::write(
        site.fixture_dir.join("sass/_variables.scss"),
        "$color: blue;",
    )
    .expect("write partial");

    // Build should succeed
    let result = site.build().await;
    result.assert_success();
}

#[tokio::test]
async fn scss_compiled_to_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    // SASS files from sass/ directory are served at /main.*.css
    let css_url = html
        .css_link("/main.*.css")
        .expect("SCSS should be compiled to /main.*.css");

    let css = site.get(&css_url);
    css.assert_ok();

    // SCSS variables should be compiled to actual values
    css.assert_contains("#3498db"); // $primary-color value
    css.assert_not_contains("$primary-color"); // Variables should be resolved
}

#[tokio::test]
async fn scss_change_triggers_rebuild() {
    let site = TestSite::new("sample-site");

    let css_url_1 = site
        .get("/")
        .css_link("/main.*.css")
        .expect("initial SCSS CSS URL");

    site.wait_debounce();

    // Modify the SCSS file
    site.modify_file("sass/main.scss", |scss| scss.replace("#3498db", "#ff0000"));

    // Wait for the CSS URL to change
    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let url = site.get("/").css_link("/main.*.css")?;
        if url != css_url_1 { Some(url) } else { None }
    });

    let css = site.get(&css_url_2);
    // SASS may optimize #ff0000 to "red"
    assert!(
        css.text().contains("#ff0000") || css.text().contains("red"),
        "CSS should have the new color: {}",
        css.text()
    );
    // Original color should be replaced
    css.assert_not_contains("#3498db");
}
