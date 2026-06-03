use super::*;

pub async fn test_new_section_detected() {
    let site = TestSite::new("sample-site");
    site.wait_debounce().await;
    site.delete_if_exists("content/new-section");

    let resp = site.get("/new-section/").await;
    assert_eq!(resp.status, 404, "New section should not exist initially");

    site.write_file(
        "content/new-section/_index.md",
        r#"+++
title = "New Section"
+++

This is a dynamically created section."#,
    );

    let _resp = site
        .wait_until(
            "new section page to be accessible",
            Duration::from_secs(2),
            async || {
                let resp = site.get("/new-section/").await;
                if resp.status == 200 { Some(resp) } else { None }
            },
        )
        .await;

    let resp = site.get("/new-section/").await;
    resp.assert_ok();
    resp.assert_contains("dynamically created section");
}

pub async fn test_deeply_nested_new_section() {
    let site = TestSite::new("sample-site");
    site.wait_debounce().await;
    site.delete_if_exists("content/level1");

    let resp = site.get("/level1/level2/level3/").await;
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

    let _resp = site
        .wait_until(
            "deeply nested section page to be accessible",
            Duration::from_secs(2),
            async || {
                let resp = site.get("/level1/level2/level3/").await;
                if resp.status == 200 { Some(resp) } else { None }
            },
        )
        .await;

    let resp = site.get("/level1/level2/level3/").await;
    resp.assert_ok();
    resp.assert_contains("deeply nested section");
}

pub async fn test_file_move_detected() {
    let site = TestSite::new("sample-site");
    site.wait_debounce().await;

    site.write_file(
        "content/guide/moveable.md",
        r#"+++
title = "Moveable Page"
+++

This page will be moved."#,
    );

    site.wait_until(
        "moved page to be accessible at new location",
        Duration::from_secs(2),
        async || {
            let resp = site.get("/guide/moveable/").await;
            if resp.status == 200 { Some(resp) } else { None }
        },
    )
    .await;

    site.wait_debounce().await;

    let original_content = site.read_file("content/guide/moveable.md");
    site.delete_file("content/guide/moveable.md");
    site.write_file("content/moved-page.md", &original_content);

    let result = site
        .wait_until(
            "old page to return 404 and new page to return 200",
            Duration::from_secs(2),
            async || {
                let old_resp = site.get("/guide/moveable/").await;
                let new_resp = site.get("/moved-page/").await;

                if old_resp.status == 404 && new_resp.status == 200 {
                    Some((old_resp, new_resp))
                } else {
                    None
                }
            },
        )
        .await;

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

pub async fn test_css_livereload() {
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
        .await
        .css_link("/css/style.*.css")
        .expect("Initial CSS URL should exist");

    let css_1 = site.get(&css_url_1).await;
    css_1.assert_contains("font-weight:400");

    site.wait_debounce().await;

    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    let css_url_2 = site
        .wait_until(
            "CSS URL to change for livereload",
            Duration::from_secs(2),
            async || {
                let new_url = site.get("/").await.css_link("/css/style.*.css")?;
                if new_url != css_url_1 {
                    Some(new_url)
                } else {
                    None
                }
            },
        )
        .await;

    let css_2 = site.get(&css_url_2).await;
    css_2.assert_contains("font-weight:700");
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification"
    );
}
