use super::*;

pub fn no_scss_builds_successfully() {
    let site = TestSite::new("no-scss-site");

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("Welcome");

    assert!(
        html.css_link("/main.*.css").is_none(),
        "No CSS should be generated when SCSS is absent"
    );
}

pub fn scss_compiled_to_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    let css_url = html
        .css_link("/main.*.css")
        .expect("SCSS should be compiled to /main.*.css");

    let css = site.get(&css_url);
    css.assert_ok();
    css.assert_contains("#3498db");
    css.assert_not_contains("$primary-color");
}

pub fn scss_change_triggers_rebuild() {
    let site = TestSite::new("sample-site");

    let css_url_1 = site
        .get("/")
        .css_link("/main.*.css")
        .expect("initial SCSS CSS URL");

    site.wait_debounce();

    site.modify_file("sass/main.scss", |scss| scss.replace("#3498db", "#ff0000"));

    let css_url_2 = site.wait_until(
        "SCSS change to trigger CSS rebuild and URL change",
        Duration::from_secs(10),
        || {
            let url = site.get("/").css_link("/main.*.css")?;
            if url != css_url_1 { Some(url) } else { None }
        },
    );

    let css = site.get(&css_url_2);
    assert!(
        css.text().contains("#ff0000") || css.text().contains("red"),
        "CSS should have the new color: {}",
        css.text()
    );
    css.assert_not_contains("#3498db");
}
