//! SASS/SCSS compilation tests

use crate::harness::TestSite;
use std::time::Duration;

#[test_log::test]
fn scss_compiled_to_css() {
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

#[test_log::test]
fn scss_change_triggers_rebuild() {
    let site = TestSite::new("sample-site");

    let css_url_1 = site
        .get("/")
        .css_link("/main.*.css")
        .expect("initial SCSS CSS URL");

    site.wait_debounce();

    // Modify the SCSS file
    site.modify_file("sass/main.scss", |scss| {
        scss.replace("#3498db", "#ff0000")
    });

    // Wait for the CSS URL to change
    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let url = site.get("/").css_link("/main.*.css")?;
        if url != css_url_1 {
            Some(url)
        } else {
            None
        }
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
