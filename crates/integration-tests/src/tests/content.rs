use super::*;

pub fn markdown_content_rendered() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("Welcome");
    html.assert_contains("This is the home page");
}

pub fn frontmatter_title_in_html() {
    let site = TestSite::new("sample-site");
    let html = site.get("/");
    html.assert_contains("<title>Home</title>");
}

pub fn nested_content_structure() {
    let site = TestSite::new("sample-site");
    site.get("/guide/").assert_ok();
    site.get("/guide/getting-started/").assert_ok();
    site.get("/guide/advanced/").assert_ok();
}

pub fn missing_page_title_defaults_from_slug() {
    let site = TestSite::with_files(
        "sample-site",
        &[("content/hello-world.md", "# Hello\n\nBody\n")],
    );

    let html = site.get("/hello-world/");
    html.assert_ok();
    html.assert_contains("<title>Hello World</title>");
}

pub fn missing_section_title_defaults_from_slug() {
    let site = TestSite::with_files(
        "sample-site",
        &[("content/hello-world/_index.md", "# Hello\n\nBody\n")],
    );

    let html = site.get("/hello-world/");
    html.assert_ok();
    html.assert_contains("<title>Hello World</title>");
}
