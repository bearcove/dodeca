use super::*;

const CONFIG: &str = r#"source {
    content content
    impls (
        {
            name rust
            include ("code/**/*.rs")
        }
    )
}

site {
    output public
}
"#;

const SPEC: &str = r#"+++
title = "Coverage Spec"
+++

# Coverage Spec

r[api.live+2] Current live rule.

r[api.todo] Unimplemented rule.
"#;

const CODE: &str = r#"// r[impl api.live+2]
// r[verify api.live+2]
// r[impl api.live]
pub fn live() {}
"#;

const MULTI_CONFIG: &str = r#"source {
    content root/content
    impls (
        {
            name rust
            include ("root-code/**/*.rs")
        }
    )
}

mounts (
    {
        name api
        path /api
        local api/content
    }
)

site {
    output public
}
"#;

const MOUNT_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name rust
            include ("code/**/*.rs")
            test_include ("tests/**/*.rs")
        }
    )
}
"#;

const SIBLING_CONFIG: &str = r#"source {
    content root/content
}

mounts (
    {
        name vix
        path /vix
        local vix/content
    }
)

site {
    output public
}
"#;

const VIX_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name weavy
            root ../weavy
            include ("src/**/*.rs")
            test_include ("tests/**/*.rs")
        }
    )
}
"#;

const ROOT_SPEC: &str = r#"+++
title = "Root Spec"
+++

r[root.rule] Root rule.
"#;

const API_SPEC: &str = r#"+++
title = "API Spec"
+++

r[api.rule] API rule.

r[api.testonly] Test-only rule.
"#;

const ROOT_CODE: &str = r#"// r[impl root.rule]
pub fn root_rule() {}
"#;

const API_CODE: &str = r#"// r[impl api.rule]
pub fn api_rule() {}

pub fn api_unmapped() {}
"#;

const API_TEST_CODE: &str = r#"// r[verify api.testonly]
pub fn api_rule_test() {}
"#;

const VIX_SPEC: &str = r#"+++
title = "Vix Spec"
+++

r[vix.weavy] Vix rule implemented in sibling Weavy.
"#;

const VIX_WATCH_SPEC: &str = r#"+++
title = "Vix Watch Spec"
+++

r[vix.weavy] Vix rule implemented in sibling Weavy.

r[vix.created] Vix rule implemented by a moved-in file.
"#;

const VIX_RELOAD_SPEC: &str = r#"+++
title = "Vix Reload Spec"
+++

r[vix.old] Old implementation root rule.

r[vix.new] New implementation root rule.
"#;

const WEAVY_CODE: &str = r#"// r[impl vix.weavy]
pub fn weavy_impl() {}
"#;

const WEAVY_TEST_CODE: &str = r#"// r[verify vix.weavy]
pub fn weavy_verify() {}
"#;

const WEAVY_CREATED_CODE: &str = r#"// r[impl vix.created]
pub fn created_impl() {}
"#;

const VIX_OLD_ROOT_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name weavy
            root ../weavy-old
            include ("src/**/*.rs")
        }
    )
}
"#;

const VIX_NEW_ROOT_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name weavy
            root ../weavy-new
            include ("src/**/*.rs")
            test_include ("tests/**/*.rs")
        }
    )
}
"#;

const WEAVY_OLD_CODE: &str = r#"// r[impl vix.old]
pub fn old_impl() {}
"#;

const WEAVY_NEW_CODE: &str = r#"// r[impl vix.new]
pub fn new_impl() {}
"#;

const WEAVY_NEW_TEST_CODE: &str = r#"// r[verify vix.new]
pub fn new_verify() {}
"#;

const PREFIX_FILTER_CONFIG: &str = r#"mounts (
    {
        name api
        path /api
        local api/content
    }
    {
        name web
        path /web
        local web/content
    }
)

site {
    output public
}
"#;

const API_PREFIX_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name shared
            root ..
            include ("shared/**/*.rs")
        }
    )
}
"#;

const WEB_PREFIX_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name shared
            root ..
            include ("shared/**/*.rs" "web-impl/**/*.rs")
        }
    )
}
"#;

const API_PREFIX_SPEC: &str = r#"+++
title = "API Prefix Spec"
+++

r[shared.api] API rule implemented from a shared file.

r[api.only] API-only rule remains uncovered.
"#;

const WEB_PREFIX_SPEC: &str = r#"+++
title = "Web Prefix Spec"
+++

http[shared.web] Web rule implemented from a shared file.

http[web.only] Web-only rule must not be covered by r-prefixed code.
"#;

const SHARED_PREFIX_CODE: &str = r#"// r[impl shared.api]
pub fn api_shared() {}

// http[impl shared.web]
pub fn web_shared() {}

// arr[k] and arr[ix] are array indexing examples, not this source's prefix.
pub fn array_lookup() {}
"#;

const WEB_WRONG_PREFIX_CODE: &str = r#"// r[impl web.only]
pub fn wrong_prefix_web_only() {}
"#;

const DUPLICATE_CONTEXT_CONFIG: &str = r#"source {
    content content
    impls (
        {
            name first
            include ("shared/**/*.rs")
        }
        {
            name second
            include ("shared/**/*.rs")
        }
    )
}

site {
    output public
}
"#;

const DUPLICATE_CONTEXT_SPEC: &str = r#"+++
title = "Duplicate Context Spec"
+++

# Duplicate Context Spec

r[same.rule] One shared implementation.
"#;

const DUPLICATE_CONTEXT_CODE: &str = r#"// r[impl same.rule]
pub fn shared_impl() {}
"#;

async fn wait_contains(site: &TestSite, path: &str, needle: &str) -> Response {
    site.wait_until(
        &format!("{path} contains {needle}"),
        Duration::from_secs(10),
        async || {
            let response = site.get(path).await;
            (response.status == 200 && response.body.contains(needle)).then_some(response)
        },
    )
    .await
}

pub async fn coverage_suffix_endpoints_serve_markdown_and_json() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", CONFIG),
            ("content/coverage.md", SPEC),
            ("code/lib.rs", CODE),
        ],
    );

    let md = site.get("/_dodeca/coverage/status.md").await;
    md.assert_ok();
    md.assert_content_type("text/markdown");
    md.assert_contains("# Coverage Status");
    md.assert_contains("| Implemented | 1/2 | 50.0% |");
    md.assert_contains("| Verified | 1/2 | 50.0% |");
    md.assert_contains("| Stale refs | 1 |");

    let json = site.get("/_dodeca/coverage/status.json").await;
    json.assert_ok();
    json.assert_content_type("application/json");
    json.assert_contains(r#""totalRules": 2"#);
    json.assert_contains(r#""implementedRules": 1"#);
    json.assert_contains(r#""staleReferences": 1"#);

    let nav = site.get("/_dodeca/coverage/nav.md").await;
    nav.assert_ok();
    nav.assert_content_type("text/markdown");
    nav.assert_contains("# Coverage Navigation");
    nav.assert_contains("## Spec View");
    nav.assert_contains("## Coverage View");
    nav.assert_contains("## Sources View");
    nav.assert_contains("[`api.live+2`](rule/api.live%2B2.md)");
    nav.assert_contains("`code/lib.rs`");

    let nav_json = site.get("/_dodeca/coverage/nav.json").await;
    nav_json.assert_ok();
    nav_json.assert_content_type("application/json");
    nav_json.assert_contains(r#""id": "spec""#);
    nav_json.assert_contains(r#""id": "coverage""#);
    nav_json.assert_contains(r#""id": "sources""#);
    nav_json.assert_contains(r#""specRoutes""#);
    nav_json.assert_contains(r#""sourceFiles""#);
    nav_json.assert_contains(r#""ruleHref": "rule/api.live%2B2.md""#);

    let nav_html = site.get("/_dodeca/coverage/nav.html").await;
    nav_html.assert_ok();
    nav_html.assert_content_type("text/html");
    nav_html.assert_contains("<h1>Coverage Navigation</h1>");
    nav_html.assert_contains("Review Queues");
    nav_html.assert_contains("Spec View");
    nav_html.assert_contains("Coverage View");
    nav_html.assert_contains("Sources View");
    nav_html.assert_contains("class=\"rule-card is-stale\"");
    nav_html.assert_contains("Current live rule.");
    nav_html.assert_contains("rule/api.live%2B2.html");

    let nav_root = site.get("/_dodeca/coverage/").await;
    nav_root.assert_ok();
    nav_root.assert_content_type("text/html");
    nav_root.assert_contains("<h1>Coverage Navigation</h1>");

    let uncovered = site.get("/_dodeca/coverage/uncovered.md").await;
    uncovered.assert_ok();
    uncovered.assert_contains("api.todo");

    let stale = site.get("/_dodeca/coverage/stale.md").await;
    stale.assert_ok();
    stale.assert_contains("api.live+2");
    stale.assert_contains("api.live");
    stale.assert_contains("code/lib.rs");

    let rule = site.get("/_dodeca/coverage/rule/api.live%2B2.md").await;
    rule.assert_ok();
    rule.assert_contains("# Rule `api.live+2`");
    rule.assert_contains("## Definitions");
    rule.assert_contains("Current live rule.");
    rule.assert_contains("## Implementation References");
    rule.assert_contains("## Verification References");
    rule.assert_contains("code/lib.rs");

    let rule_html = site.get("/_dodeca/coverage/rule/api.live%2B2.html").await;
    rule_html.assert_ok();
    rule_html.assert_content_type("text/html");
    rule_html.assert_contains("<h1>Rule <code>api.live+2</code></h1>");
    rule_html.assert_contains("href=\"../nav.html\"");
    rule_html.assert_contains("href=\"api.live%2B2.md\"");
    rule_html.assert_contains("href=\"api.live%2B2.json\"");
    rule_html.assert_contains("Current live rule.");
    rule_html.assert_contains("Implementation References");
    rule_html.assert_contains("Verification References");
    rule_html.assert_contains("Stale References");
    rule_html.assert_contains("<code>code/lib.rs</code>:1");
    rule_html.assert_contains("api.live");
}

pub async fn coverage_filters_by_source_and_impl() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", MULTI_CONFIG),
            ("root/content/root.md", ROOT_SPEC),
            ("root-code/lib.rs", ROOT_CODE),
            ("api/.config/dodeca.styx", MOUNT_CONFIG),
            ("api/content/api.md", API_SPEC),
            ("api/code/lib.rs", API_CODE),
            ("api/tests/api_test.rs", API_TEST_CODE),
        ],
    );

    let api = site
        .get("/_dodeca/coverage/status.md?source=api&impl=rust")
        .await;
    api.assert_ok();
    api.assert_contains("Spec: `api/rust`");
    api.assert_contains("| Implemented | 1/2 | 50.0% |");
    api.assert_contains("| Verified | 1/2 | 50.0% |");
    api.assert_contains("| Test impl refs | 0 |");

    let api_json = site
        .get("/_dodeca/coverage/status.json?source=api&impl=rust")
        .await;
    api_json.assert_ok();
    api_json.assert_contains(r#""specName": "api/rust""#);
    api_json.assert_contains(r#""totalRules": 2"#);
    api_json.assert_contains(r#""implementedRules": 1"#);
    api_json.assert_contains(r#""verifiedRules": 1"#);
    api_json.assert_contains(r#""testImplReferences": 0"#);

    let config = site
        .get("/_dodeca/coverage/config.md?source=api&impl=rust")
        .await;
    config.assert_ok();
    config.assert_contains("# Coverage Config");
    config.assert_contains("## `api` / `rust`");
    config.assert_contains("- Root: default source project root");
    config.assert_contains("code/**/*.rs");
    config.assert_contains("tests/**/*.rs");

    let config_json = site
        .get("/_dodeca/coverage/config.json?source=api&impl=rust")
        .await;
    config_json.assert_ok();
    config_json.assert_contains(r#""implName": "rust""#);
    config_json.assert_contains(r#""sourceName": "api""#);
    config_json.assert_contains("code/**/*.rs");
    config_json.assert_contains("tests/**/*.rs");

    let validate = site
        .get("/_dodeca/coverage/validate.md?source=api&impl=rust")
        .await;
    validate.assert_ok();
    validate.assert_contains("Result: **passing**");
    validate.assert_contains("- Test impl references: `0`");

    let unmapped = site
        .get("/_dodeca/coverage/unmapped.md?source=api&impl=rust")
        .await;
    unmapped.assert_ok();
    unmapped.assert_contains("# Unmapped Code Units");
    unmapped.assert_contains("api_unmapped");
    unmapped.assert_contains("api/code/lib.rs");

    let unmapped_json = site
        .get("/_dodeca/coverage/unmapped.json?source=api&impl=rust")
        .await;
    unmapped_json.assert_ok();
    unmapped_json.assert_contains(r#""name": "api_unmapped""#);
    unmapped_json.assert_contains(r#""file": "api/code/lib.rs""#);

    let missing = site
        .get("/_dodeca/coverage/status.md?source=api&impl=go")
        .await;
    assert_eq!(missing.status, 404);
}

pub async fn coverage_filters_refs_by_source_marker_prefix() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", PREFIX_FILTER_CONFIG),
            ("api/.config/dodeca.styx", API_PREFIX_CONFIG),
            ("api/content/api.md", API_PREFIX_SPEC),
            ("web/.config/dodeca.styx", WEB_PREFIX_CONFIG),
            ("web/content/web.md", WEB_PREFIX_SPEC),
            ("shared/src/lib.rs", SHARED_PREFIX_CODE),
            ("web-impl/wrong.rs", WEB_WRONG_PREFIX_CODE),
        ],
    );

    let api_status = site
        .get("/_dodeca/coverage/status.md?source=api&impl=shared")
        .await;
    api_status.assert_ok();
    api_status.assert_contains("Spec: `api/shared`");
    api_status.assert_contains("| Implemented | 1/2 | 50.0% |");
    api_status.assert_contains("| Invalid refs | 0 |");

    let api_rule = site
        .get("/_dodeca/coverage/rule/shared.api.md?source=api&impl=shared")
        .await;
    api_rule.assert_ok();
    api_rule.assert_contains("shared/src/lib.rs");

    let api_uncovered = site
        .get("/_dodeca/coverage/uncovered.md?source=api&impl=shared")
        .await;
    api_uncovered.assert_ok();
    api_uncovered.assert_contains("api.only");

    let api_unmapped = site
        .get("/_dodeca/coverage/unmapped.md?source=api&impl=shared")
        .await;
    api_unmapped.assert_ok();
    api_unmapped.assert_contains("array_lookup");
    api_unmapped.assert_contains("web_shared");

    let web_status = site
        .get("/_dodeca/coverage/status.md?source=web&impl=shared")
        .await;
    web_status.assert_ok();
    web_status.assert_contains("Spec: `web/shared`");
    web_status.assert_contains("| Implemented | 1/2 | 50.0% |");
    web_status.assert_contains("| Invalid refs | 0 |");

    let web_rule = site
        .get("/_dodeca/coverage/rule/shared.web.md?source=web&impl=shared")
        .await;
    web_rule.assert_ok();
    web_rule.assert_contains("shared/src/lib.rs");

    let web_uncovered = site
        .get("/_dodeca/coverage/uncovered.md?source=web&impl=shared")
        .await;
    web_uncovered.assert_ok();
    web_uncovered.assert_contains("web.only");

    let web_unmapped = site
        .get("/_dodeca/coverage/unmapped.md?source=web&impl=shared")
        .await;
    web_unmapped.assert_ok();
    web_unmapped.assert_contains("array_lookup");
    web_unmapped.assert_contains("api_shared");
    web_unmapped.assert_contains("wrong_prefix_web_only");
}

pub async fn coverage_deduplicates_shared_files_across_impl_contexts() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", DUPLICATE_CONTEXT_CONFIG),
            ("content/coverage.md", DUPLICATE_CONTEXT_SPEC),
            ("shared/src/lib.rs", DUPLICATE_CONTEXT_CODE),
        ],
    );

    let status = site.get("/_dodeca/coverage/status.json").await;
    status.assert_ok();
    assert_eq!(status.body.matches(r#""implRefs": 1"#).count(), 1);

    let rule = site.get("/_dodeca/coverage/rule/same.rule.json").await;
    rule.assert_ok();
    assert_eq!(
        rule.body.matches(r#""file": "shared/src/lib.rs""#).count(),
        1
    );

    let rendered = site.get("/coverage/").await;
    rendered.assert_ok();
    assert_eq!(
        rendered
            .body
            .matches("function shared_impl — shared/src/lib.rs:")
            .count(),
        1
    );
}

pub async fn coverage_impl_root_scans_sibling_crate() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", SIBLING_CONFIG),
            ("root/content/root.md", ROOT_SPEC),
            ("vix/.config/dodeca.styx", VIX_CONFIG),
            ("vix/content/vix.md", VIX_SPEC),
            ("weavy/src/lib.rs", WEAVY_CODE),
            ("weavy/tests/weavy_test.rs", WEAVY_TEST_CODE),
        ],
    );

    let status = site
        .get("/_dodeca/coverage/status.md?source=vix&impl=weavy")
        .await;
    status.assert_ok();
    status.assert_contains("Spec: `vix/weavy`");
    status.assert_contains("| Implemented | 1/1 | 100.0% |");
    status.assert_contains("| Verified | 1/1 | 100.0% |");

    let config = site
        .get("/_dodeca/coverage/config.md?source=vix&impl=weavy")
        .await;
    config.assert_ok();
    config.assert_contains("## `vix` / `weavy`");
    config.assert_contains("- Root: `../weavy`");
    config.assert_contains("src/**/*.rs");
    config.assert_contains("tests/**/*.rs");

    let rule = site
        .get("/_dodeca/coverage/rule/vix.weavy.md?source=vix&impl=weavy")
        .await;
    rule.assert_ok();
    rule.assert_contains("## Implementation References");
    rule.assert_contains("## Verification References");
    rule.assert_contains("weavy/src/lib.rs");
    rule.assert_contains("weavy/tests/weavy_test.rs");

    let nav = site
        .get("/_dodeca/coverage/nav.md?source=vix&impl=weavy")
        .await;
    nav.assert_ok();
    nav.assert_contains("`weavy/src/lib.rs`");
    nav.assert_contains("`weavy/tests/weavy_test.rs`");
}

pub async fn coverage_watcher_recomputes_sibling_impl_root_files() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", SIBLING_CONFIG),
            ("root/content/root.md", ROOT_SPEC),
            ("vix/.config/dodeca.styx", VIX_CONFIG),
            ("vix/content/vix.md", VIX_WATCH_SPEC),
            ("weavy/src/lib.rs", WEAVY_CODE),
            ("weavy/tests/weavy_test.rs", WEAVY_TEST_CODE),
        ],
    );

    let status_path = "/_dodeca/coverage/status.md?source=vix&impl=weavy";
    wait_contains(&site, status_path, "| Implemented | 1/2 | 50.0% |").await;
    wait_contains(&site, status_path, "| Verified | 1/2 | 50.0% |").await;

    site.modify_file("weavy/src/lib.rs", |_| {
        "pub fn weavy_impl() {}\n".to_string()
    });
    wait_contains(&site, status_path, "| Implemented | 0/2 | 0.0% |").await;

    site.write_file("weavy/src/lib.rs", WEAVY_CODE);
    wait_contains(&site, status_path, "| Implemented | 1/2 | 50.0% |").await;

    site.write_file("scratch/created.rs", WEAVY_CREATED_CODE);
    std::fs::rename(
        site.fixture_dir().join("scratch/created.rs"),
        site.fixture_dir().join("weavy/src/created.rs"),
    )
    .expect("move created impl into sibling root");
    wait_contains(&site, status_path, "| Implemented | 2/2 | 100.0% |").await;

    let created_rule = site
        .get("/_dodeca/coverage/rule/vix.created.md?source=vix&impl=weavy")
        .await;
    created_rule.assert_ok();
    created_rule.assert_contains("weavy/src/created.rs");

    std::fs::rename(
        site.fixture_dir().join("weavy/src/created.rs"),
        site.fixture_dir().join("scratch/created.rs"),
    )
    .expect("move created impl out of sibling root");
    wait_contains(&site, status_path, "| Implemented | 1/2 | 50.0% |").await;

    std::fs::rename(
        site.fixture_dir().join("scratch/created.rs"),
        site.fixture_dir().join("weavy/src/created.rs"),
    )
    .expect("re-add created impl to sibling root");
    wait_contains(&site, status_path, "| Implemented | 2/2 | 100.0% |").await;
}

pub async fn coverage_config_reload_rewatches_changed_sibling_impl_root() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", SIBLING_CONFIG),
            ("root/content/root.md", ROOT_SPEC),
            ("vix/.config/dodeca.styx", VIX_OLD_ROOT_CONFIG),
            ("vix/content/vix.md", VIX_RELOAD_SPEC),
            ("weavy-old/src/lib.rs", WEAVY_OLD_CODE),
            ("weavy-new/src/lib.rs", WEAVY_NEW_CODE),
            ("weavy-new/tests/weavy_test.rs", WEAVY_NEW_TEST_CODE),
        ],
    );

    let status_path = "/_dodeca/coverage/status.md?source=vix&impl=weavy";
    wait_contains(&site, status_path, "| Implemented | 1/2 | 50.0% |").await;

    let old_rule = site
        .get("/_dodeca/coverage/rule/vix.old.md?source=vix&impl=weavy")
        .await;
    old_rule.assert_ok();
    old_rule.assert_contains("weavy-old/src/lib.rs");

    site.write_file("vix/.config/dodeca.styx", VIX_NEW_ROOT_CONFIG);
    wait_contains(&site, status_path, "| Verified | 1/2 | 50.0% |").await;

    let config = site
        .get("/_dodeca/coverage/config.md?source=vix&impl=weavy")
        .await;
    config.assert_ok();
    config.assert_contains("- Root: `../weavy-new`");
    config.assert_contains("src/**/*.rs");
    config.assert_contains("tests/**/*.rs");

    let new_rule = site
        .get("/_dodeca/coverage/rule/vix.new.md?source=vix&impl=weavy")
        .await;
    new_rule.assert_ok();
    new_rule.assert_contains("weavy-new/src/lib.rs");
    new_rule.assert_contains("weavy-new/tests/weavy_test.rs");

    site.modify_file("weavy-new/src/lib.rs", |_| {
        "pub fn new_impl() {}\n".to_string()
    });
    wait_contains(&site, status_path, "| Implemented | 0/2 | 0.0% |").await;

    site.write_file("weavy-new/src/lib.rs", WEAVY_NEW_CODE);
    wait_contains(&site, status_path, "| Implemented | 1/2 | 50.0% |").await;
}

pub async fn coverage_serve_rejects_duplicate_impl_names() {
    let temp = tempfile::Builder::new()
        .prefix("dodeca-duplicate-impls-")
        .tempdir()
        .expect("create duplicate impl fixture");
    let root = temp.path();
    std::fs::create_dir_all(root.join(".config")).expect("create config dir");
    std::fs::create_dir_all(root.join("content")).expect("create content dir");
    std::fs::write(root.join("content/spec.md"), VIX_SPEC).expect("write spec");
    std::fs::write(
        root.join(".config/dodeca.styx"),
        r#"source {
    content content
    impls (
        {
            name rust
            include ("src/**/*.rs")
        }
        {
            name rust
            test_include ("tests/**/*.rs")
        }
    )
}

site {
    output public
}
"#,
    )
    .expect("write duplicate impl config");

    let mut child = std::process::Command::new(ddc_binary())
        .arg("serve")
        .arg(root)
        .arg("--no-tui")
        .env("DODECA_QUIET", "1")
        .env("RUST_BACKTRACE", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn ddc serve");
    let mut exited = false;
    for _ in 0..20 {
        if child.try_wait().expect("poll ddc serve").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !exited {
        let _ = child.kill();
    }
    let output = child.wait_with_output().expect("collect ddc serve output");
    assert!(exited, "duplicate impl serve did not exit promptly");
    assert!(
        !output.status.success(),
        "duplicate impl serve should fail, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("duplicate impl name `rust`"),
        "duplicate impl error missing, output:\n{combined}"
    );
}
