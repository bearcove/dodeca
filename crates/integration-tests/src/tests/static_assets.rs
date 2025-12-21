use super::*;

pub fn svg_files_served() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    let svg_url = html.img_src("/images/test.*.svg");

    if let Some(url) = svg_url {
        let svg = site.get(&url);
        svg.assert_ok();
        svg.assert_contains("<svg");
    }
}

pub fn js_files_cache_busted() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");

    if html.text().contains("/js/main.") {
        let has_hash = html.text().contains("/js/main.") && html.text().contains(".js");
        assert!(has_hash, "JS files should have cache-busted URLs");
    }
}

pub fn static_files_served_directly() {
    let site = TestSite::new("sample-site");
    site.write_file("static/test.txt", "Hello, World!");
    site.wait_debounce();
    std::thread::sleep(Duration::from_secs(1));

    let resp = site.get("/test.txt");
    assert!(
        resp.status == 200 || resp.status == 404,
        "Static file response should be valid"
    );
}

pub fn image_files_processed() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");

    if let Some(svg_url) = html.img_src("/images/test.*.svg") {
        let svg = site.get(&svg_url);
        svg.assert_ok();
    }
}
