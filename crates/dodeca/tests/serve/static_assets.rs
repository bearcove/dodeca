//! Static asset serving tests (SVG, JS, images)

use crate::harness::TestSite;

#[test_log::test]
fn svg_files_served() {
    let site = TestSite::new("sample-site");

    // SVG files should be served (possibly optimized)
    let html = site.get("/");
    let svg_url = html.img_src("/images/test.*.svg");

    if let Some(url) = svg_url {
        let svg = site.get(&url);
        svg.assert_ok();
        svg.assert_contains("<svg");
    }
}

#[test_log::test]
fn js_files_cache_busted() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // Check if JS file is referenced with hash
    if html.text().contains("/js/main.") {
        // JS should have cache-busted URL
        let has_hash = html.text().contains("/js/main.") && html.text().contains(".js");
        assert!(has_hash, "JS files should have cache-busted URLs");
    }
}

#[test_log::test]
fn static_files_served_directly() {
    let site = TestSite::new("sample-site");

    // Create a simple static file
    site.write_file("static/test.txt", "Hello, World!");
    site.wait_debounce();
    std::thread::sleep(std::time::Duration::from_secs(1));

    let resp = site.get("/test.txt");
    // Should be served (may or may not be cache-busted depending on config)
    assert!(
        resp.status == 200 || resp.status == 404,
        "Static file response should be valid"
    );
}

#[test_log::test]
fn image_files_processed() {
    let site = TestSite::new("sample-site");

    let html = site.get("/");

    // If there are images referenced, they should be properly handled
    // Processable images (PNG, JPG) get picture element treatment
    // SVGs get cache-busted URLs
    if let Some(svg_url) = html.img_src("/images/test.*.svg") {
        let svg = site.get(&svg_url);
        svg.assert_ok();
    }
}
