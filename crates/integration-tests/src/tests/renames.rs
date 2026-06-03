use super::*;

pub async fn dev_server_rename_prunes_old_content_route() {
    let site = TestSite::new("sample-site");
    site.wait_debounce().await;
    site.delete_if_exists("content/rename-source.md");
    site.delete_if_exists("content/rename-target.md");

    site.write_file(
        "content/rename-source.md",
        r#"+++
title = "Rename Source"
+++

This page is about to be renamed."#,
    );

    site.wait_until(
        "source page to be accessible before rename",
        Duration::from_secs(2),
        async || {
            let resp = site.get("/rename-source/").await;
            if resp.status == 200 { Some(resp) } else { None }
        },
    )
    .await;

    std::fs::rename(
        site.fixture_dir().join("content/rename-source.md"),
        site.fixture_dir().join("content/rename-target.md"),
    )
    .expect("rename content file");

    let (old_resp, new_resp) = site
        .wait_until(
            "old route to disappear and new route to appear after rename",
            Duration::from_secs(3),
            async || {
                let old_resp = site.get("/rename-source/").await;
                let new_resp = site.get("/rename-target/").await;
                if old_resp.status == 404 && new_resp.status == 200 {
                    Some((old_resp, new_resp))
                } else {
                    None
                }
            },
        )
        .await;

    assert_eq!(old_resp.status, 404);
    assert_eq!(new_resp.status, 200);
    new_resp.assert_contains("This page is about to be renamed");
}

pub async fn build_rename_removes_old_content_output() {
    let site = InlineSite::new(&[(
        "infra.md",
        r#"+++
title = "Infra"
+++

Old route."#,
    )]);

    site.build_in_place().assert_success();
    assert!(site.fixture_dir.join("public/infra/index.html").exists());

    std::fs::rename(
        site.fixture_dir.join("content/infra.md"),
        site.fixture_dir.join("content/cluster.md"),
    )
    .expect("rename content file");

    site.build_in_place().assert_success();

    assert!(
        !site.fixture_dir.join("public/infra/index.html").exists(),
        "old output route should be removed after content rename"
    );
    assert!(
        site.fixture_dir.join("public/cluster/index.html").exists(),
        "new output route should exist after content rename"
    );
}
