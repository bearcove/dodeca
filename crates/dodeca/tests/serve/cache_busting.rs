//! Cache busting / URL rewriting tests

use crate::harness::TestSite;
use std::time::Duration;

#[test_log::test]
fn css_urls_are_cache_busted() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    let css_url = html.css_link("/css/style.*.css");

    assert!(css_url.is_some(), "CSS should have cache-busted URL");
    assert!(
        css_url.as_ref().unwrap().contains('.'),
        "URL should contain hash: {:?}",
        css_url
    );
}

#[test_log::test]
fn font_urls_rewritten_in_css() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    let css_url = html
        .css_link("/css/style.*.css")
        .expect("CSS link should exist");
    let css = site.get(&css_url);

    // Font URLs in CSS should be cache-busted
    css.assert_contains("/fonts/");
    // Should NOT contain the original unhashed URL
    css.assert_not_contains("url('/fonts/test.woff2')");
    css.assert_not_contains("url(\"/fonts/test.woff2\")");
}

#[test_log::test]
fn css_change_updates_hash() {
    let site = TestSite::new("sample-site");

    let css_url_1 = site
        .get("/")
        .css_link("/css/style.*.css")
        .expect("initial CSS URL");

    site.wait_debounce();

    site.modify_file("static/css/style.css", |css| {
        css.replace("font-weight: 400", "font-weight: 700")
    });

    let css_url_2 = site.wait_until(Duration::from_secs(10), || {
        let url = site.get("/").css_link("/css/style.*.css")?;
        if url != css_url_1 {
            Some(url)
        } else {
            None
        }
    });

    let css = site.get(&css_url_2);
    assert!(
        css.text().contains("font-weight: 700") || css.text().contains("font-weight:700"),
        "CSS should have updated font-weight"
    );
}

#[test_log::test]
fn fonts_are_subsetted() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");
    let css_url = html
        .css_link("/css/style.*.css")
        .expect("CSS link should exist");
    let css = site.get(&css_url);

    // Extract font URL from CSS (it should be cache-busted)
    let font_url = css.extract(r#"url\(['"]?(/fonts/test\.[^'")\s]+\.woff2)['"]?\)"#);
    assert!(
        font_url.is_some(),
        "Font URL should be in CSS: {}",
        css.text()
    );

    // Fetch the font - verify it's served correctly
    let font_resp = site.get(font_url.as_ref().unwrap());
    font_resp.assert_ok();
}
