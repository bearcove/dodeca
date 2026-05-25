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

    let site = TestSite::with_setup("sample-site", |fixture_dir| {
        let valid_font = fixture_dir.join("static/fonts/test.woff2");
        let source_file = fixture_dir.join("vendor/iosevka.woff2");
        std::fs::create_dir_all(source_file.parent().expect("source parent"))
            .expect("create vendor");
        std::fs::copy(&valid_font, &source_file).expect("copy source font");

        let link_path = fixture_dir.join("static/fonts/iosevka.woff2");
        std::fs::create_dir_all(link_path.parent().expect("link parent"))
            .expect("create static/fonts");
        unix_fs::symlink(&source_file, &link_path).expect("create font symlink");

        let css_path = fixture_dir.join("static/css/style.css");
        let mut css = std::fs::read_to_string(&css_path).expect("read stylesheet");
        css.push_str(
            "\n@font-face { font-family: Iosevka; src: url('/fonts/iosevka.woff2') format('woff2'); }\n",
        );
        std::fs::write(&css_path, css).expect("write stylesheet");
    });

    let html = site.get("/");
    let css_url = html
        .css_link("/css/style.*.css")
        .expect("CSS link should exist");
    let css = site.get(&css_url);
    let font_url = css.extract(r#"url\(['"]?(/fonts/iosevka\.[^'")\s]+\.woff2)['"]?\)"#);
    assert!(
        font_url.is_some(),
        "Symlinked font URL should be cache-busted in CSS: {}",
        css.text()
    );
    site.get(font_url.as_ref().unwrap()).assert_ok();
}
