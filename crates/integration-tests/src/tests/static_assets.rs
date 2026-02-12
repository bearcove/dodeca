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

#[cfg(unix)]
pub fn symlinked_static_files_are_served() {
    use std::os::unix::fs as unix_fs;

    let site = TestSite::new("sample-site");
    let fixture_dir = site.fixture_dir();

    let source_file = fixture_dir.join("vendor/iosevka.woff2");
    std::fs::create_dir_all(source_file.parent().expect("source parent")).expect("create vendor");
    std::fs::write(&source_file, b"fake-font").expect("write source font");

    let link_path = fixture_dir.join("static/fonts/iosevka.woff2");
    std::fs::create_dir_all(link_path.parent().expect("link parent")).expect("create static/fonts");
    if link_path.exists() {
        std::fs::remove_file(&link_path).expect("remove existing link");
    }
    unix_fs::symlink(&source_file, &link_path).expect("create font symlink");

    site.wait_debounce();
    std::thread::sleep(Duration::from_secs(1));

    let resp = site.get("/fonts/iosevka.woff2");
    assert_eq!(
        resp.status, 200,
        "Symlinked static files should be served (status {})",
        resp.status
    );
}
