//! Basic serving tests

use crate::harness::TestSite;

#[test_log::test]
fn all_pages_return_200() {
    let site = TestSite::new("sample-site");

    site.get("/").assert_ok();
    site.get("/guide/").assert_ok();
    site.get("/guide/getting-started/").assert_ok();
    site.get("/guide/advanced/").assert_ok();
}

#[test_log::test]
fn nonexistent_page_returns_404() {
    let site = TestSite::new("sample-site");

    let resp = site.get("/this-page-does-not-exist/");
    assert_eq!(resp.status, 404, "Nonexistent page should return 404");
}

#[test_log::test]
fn nonexistent_static_returns_404() {
    let site = TestSite::new("sample-site");

    let resp = site.get("/images/nonexistent.png");
    assert_eq!(resp.status, 404, "Nonexistent static file should return 404");
}

#[test_log::test]
fn pagefind_files_served() {
    let site = TestSite::new("sample-site");

    // Pagefind is built asynchronously, so wait for it
    site.wait_for("/pagefind/pagefind.js", std::time::Duration::from_secs(30))
        .assert_ok();
}
