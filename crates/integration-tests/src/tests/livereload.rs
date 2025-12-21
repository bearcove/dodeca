use super::*;

pub fn test_new_section_detected() {
    let site = TestSite::new("sample-site");
    site.wait_debounce();
    site.delete_if_exists("content/new-section");

    let resp = site.get("/new-section/");
    assert_eq!(resp.status, 404, "New section should not exist initially");

    site.write_file(
        "content/new-section/_index.md",
        r#"+++
title = "New Section"
+++

This is a dynamically created section."#,
    );

    let _resp = site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/new-section/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    let resp = site.get("/new-section/");
    resp.assert_ok();
    resp.assert_contains("dynamically created section");
}

pub fn test_deeply_nested_new_section() {
    let site = TestSite::new("sample-site");
    site.wait_debounce();
    site.delete_if_exists("content/level1");

    let resp = site.get("/level1/level2/level3/");
    assert_eq!(
        resp.status, 404,
        "Nested section should not exist initially"
    );

    site.write_file(
        "content/level1/level2/level3/_index.md",
        r#"+++
title = "Deeply Nested"
+++

This is a deeply nested section at level 3."#,
    );

    let _resp = site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/level1/level2/level3/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    let resp = site.get("/level1/level2/level3/");
    resp.assert_ok();
    resp.assert_contains("deeply nested section");
}

pub fn test_file_move_detected() {
    let site = TestSite::new("sample-site");
    site.wait_debounce();

    site.write_file(
        "content/guide/moveable.md",
        r#"+++
title = "Moveable Page"
+++

This page will be moved."#,
    );

    site.wait_until(Duration::from_secs(10), || {
        let resp = site.get("/guide/moveable/");
        if resp.status == 200 { Some(resp) } else { None }
    });

    site.wait_debounce();

    let original_content = site.read_file("content/guide/moveable.md");
    site.delete_file("content/guide/moveable.md");
    site.write_file("content/moved-page.md", &original_content);

    let result = site.wait_until(Duration::from_secs(10), || {
        let old_resp = site.get("/guide/moveable/");
        let new_resp = site.get("/moved-page/");

        if old_resp.status == 404 && new_resp.status == 200 {
            Some((old_resp, new_resp))
        } else {
            None
        }
    });

    let (old_resp, new_resp) = result;
    assert_eq!(
        old_resp.status, 404,
        "Old URL should return 404 after file move"
    );
    assert_eq!(
        new_resp.status, 200,
        "New URL should be accessible after file move"
    );
    new_resp.assert_contains("This page will be moved");
}

pub fn test_css_livereload() {
    let site = TestSite::new("sample-site");

    const BASELINE_CSS: &str = r#"/* Test CSS with font URLs */
@font-face {
    font-family: 'TestFont';
    src: url('/fonts/test.woff2') format('woff2');
    font-weight: 400;
    font-style: normal;
}

body {
    font-family: 'TestFont', sans-serif;
}
"#;
    site.write_file("static/css/style.css", BASELINE_CSS);

    let css_url_1 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("Initial CSS URL should exist");

    let css_1 = site.get(&css_url_1);
    css_1.assert_contains("font-weight:400");

    site.wait_debounce();

    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let new_url = site.get("/").css_link("/css/style.*.css")?;
        if new_url != css_url_1 {
            Some(new_url)
        } else {
            None
        }
    });

    let css_2 = site.get(&css_url_2);
    css_2.assert_contains("font-weight:700");
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification"
    );
}
