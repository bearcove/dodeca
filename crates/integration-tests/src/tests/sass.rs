use super::*;

pub async fn no_scss_builds_successfully() {
    let site = TestSite::new("no-scss-site");

    let html = site.get("/").await;
    html.assert_ok();
    html.assert_contains("Welcome");

    assert!(
        html.css_link("/main.*.css").is_none(),
        "No CSS should be generated when SCSS is absent"
    );
}

pub async fn scss_compiled_to_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/").await;
    let css_url = html
        .css_link("/main.*.css")
        .expect("SCSS should be compiled to /main.*.css");

    let css = site.get(&css_url).await;
    css.assert_ok();
    css.assert_contains("#3498db");
    css.assert_not_contains("$primary-color");
}

pub async fn scss_can_import_from_node_modules() {
    let site = TestSite::with_setup("sample-site", |fixture_dir| {
        let package_dir = fixture_dir.join("node_modules/pico");
        std::fs::create_dir_all(&package_dir).expect("create package dir");
        std::fs::write(package_dir.join("_index.scss"), "$pico-color: #c0ffee;\n")
            .expect("write package stylesheet");

        std::fs::write(
            fixture_dir.join("sass/main.scss"),
            r#"@use "pico";

.node-modules-import {
    color: pico.$pico-color;
}
"#,
        )
        .expect("write main stylesheet");
    });

    let html = site.get("/").await;
    let css_url = html
        .css_link("/main.*.css")
        .expect("SCSS should be compiled to /main.*.css");

    let css = site.get(&css_url).await;
    css.assert_ok();
    css.assert_contains("#c0ffee");
}

pub async fn scss_change_triggers_rebuild() {
    let site = TestSite::new("sample-site");

    let css_url_1 = site
        .get("/")
        .await
        .css_link("/main.*.css")
        .expect("initial SCSS CSS URL");

    site.wait_debounce().await;

    site.modify_file("sass/main.scss", |scss| scss.replace("#3498db", "#ff0000"));

    let css_url_2 = site
        .wait_until(
            "SCSS change to trigger CSS rebuild and URL change",
            Duration::from_secs(2),
            async || {
                let url = site.get("/").await.css_link("/main.*.css")?;
                if url != css_url_1 { Some(url) } else { None }
            },
        )
        .await;

    let css = site.get(&css_url_2).await;
    assert!(
        css.text().contains("#ff0000") || css.text().contains("red"),
        "CSS should have the new color: {}",
        css.text()
    );
    css.assert_not_contains("#3498db");
}
