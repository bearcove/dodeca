use super::*;

pub async fn all_pages_return_200() {
    tracing::info!("Starting all_pages_return_200 test");
    let site = TestSite::new("sample-site");
    tracing::debug!("Created test site, now testing pages");
    site.get("/").await.assert_ok();
    site.get("/guide/").await.assert_ok();
    site.get("/guide/getting-started/").await.assert_ok();
    site.get("/guide/advanced/").await.assert_ok();
}

pub async fn nonexistent_page_returns_404() {
    let site = TestSite::new("sample-site");
    let resp = site.get("/this-page-does-not-exist/").await;
    assert_eq!(resp.status, 404, "Nonexistent page should return 404");
}

pub async fn nonexistent_static_returns_404() {
    let site = TestSite::new("sample-site");
    let resp = site.get("/images/nonexistent.png").await;
    assert_eq!(
        resp.status, 404,
        "Nonexistent static file should return 404"
    );
}
