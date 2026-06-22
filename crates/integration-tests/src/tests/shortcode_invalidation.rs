use super::*;

/// Editing a page's markdown triggers re-render of that page.
pub async fn page_edit_rerenders_that_page() {
    let site = TestSite::new("shortcode-site");

    let resp = site.get("/uses-tip/").await;
    resp.assert_ok();
    resp.assert_contains("sc-tip");

    site.write_file(
        "content/uses-tip.md",
        r#"+++
title = "Uses Tip"
+++

> *:tip*
>
> This is inside the tip shortcode.

unique-marker-alpha-1234
"#,
    );

    site.wait_until(
        "uses-tip page to show new marker",
        Duration::from_secs(5),
        async || {
            let resp = site.get("/uses-tip/").await;
            if resp.body.contains("unique-marker-alpha-1234") {
                Some(resp)
            } else {
                None
            }
        },
    )
    .await;
}

/// Editing a shortcode template invalidates all pages that use it.
pub async fn shortcode_template_edit_rerenders_using_page() {
    let site = TestSite::new("shortcode-site");

    let resp = site.get("/uses-tip/").await;
    resp.assert_ok();
    resp.assert_contains("data-sentinel=\"v1\"");

    site.write_file(
        "templates/shortcodes/tip.html",
        r#"<div class="sc-tip" data-sentinel="v2">{{ body }}</div>
"#,
    );

    site.wait_until(
        "uses-tip page to show v2 sentinel",
        Duration::from_secs(5),
        async || {
            let resp = site.get("/uses-tip/").await;
            if resp.body.contains("data-sentinel=\"v2\"") {
                Some(resp)
            } else {
                None
            }
        },
    )
    .await;
}

/// Over-invalidation from shortcode template edit is benign: unrelated pages
/// still render correctly after the re-render.
pub async fn shortcode_template_edit_does_not_corrupt_unrelated_page() {
    let site = TestSite::new("shortcode-site");

    let resp = site.get("/plain-page/").await;
    resp.assert_ok();
    resp.assert_contains("plain-page-text");

    site.write_file(
        "templates/shortcodes/tip.html",
        r#"<div class="sc-tip" data-sentinel="v3">{{ body }}</div>
"#,
    );

    // Wait for the debounce window and an extra moment for the re-render to complete.
    site.wait_debounce().await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let resp = site.get("/plain-page/").await;
    resp.assert_ok();
    resp.assert_contains("plain-page-text");
}

/// Editing macros.html propagates through the dependency chain to pages that
/// import it via their shortcode template.
pub async fn macros_edit_rerenders_shortcode_importing_it() {
    let site = TestSite::new("shortcode-site");

    let resp = site.get("/uses-youtube/").await;
    resp.assert_ok();
    resp.assert_contains("data-macros-sentinel=\"v1\"");

    site.write_file(
        "templates/macros.html",
        r#"{% macro youtube_embed(id, alt="") %}<div class="youtube-embed" data-macros-sentinel="v2" data-id="{{ id }}">{{ alt }}</div>{% endmacro %}
"#,
    );

    site.wait_until(
        "uses-youtube page to show v2 macros sentinel",
        Duration::from_secs(5),
        async || {
            let resp = site.get("/uses-youtube/").await;
            if resp.body.contains("data-macros-sentinel=\"v2\"") {
                Some(resp)
            } else {
                None
            }
        },
    )
    .await;
}

// TODO: get_media not implemented yet, stub for future.
// This test will exercise the case where a media asset referenced by a
// shortcode template (via get_media) changes and should trigger re-render of
// pages using that shortcode.
pub async fn get_media_asset_rerenders_using_page() {
    // intentionally empty stub
}
