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
            include ("api/code/**/*.rs")
            test_include ("api/tests/**/*.rs")
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

const API_TEST_CODE: &str = r#"// r[impl api.testonly]
pub fn api_rule_test() {}
"#;

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
    nav_html.assert_contains("Spec View");
    nav_html.assert_contains("Coverage View");
    nav_html.assert_contains("Sources View");
    nav_html.assert_contains("rule/api.live%2B2.md");

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
    api.assert_contains("| Verified | 0/2 | 0.0% |");
    api.assert_contains("| Test impl refs | 1 |");

    let api_json = site
        .get("/_dodeca/coverage/status.json?source=api&impl=rust")
        .await;
    api_json.assert_ok();
    api_json.assert_contains(r#""specName": "api/rust""#);
    api_json.assert_contains(r#""totalRules": 2"#);
    api_json.assert_contains(r#""implementedRules": 1"#);
    api_json.assert_contains(r#""verifiedRules": 0"#);
    api_json.assert_contains(r#""testImplReferences": 1"#);

    let config = site
        .get("/_dodeca/coverage/config.md?source=api&impl=rust")
        .await;
    config.assert_ok();
    config.assert_contains("# Coverage Config");
    config.assert_contains("## `api` / `rust`");
    config.assert_contains("api/code/**/*.rs");
    config.assert_contains("api/tests/**/*.rs");

    let config_json = site
        .get("/_dodeca/coverage/config.json?source=api&impl=rust")
        .await;
    config_json.assert_ok();
    config_json.assert_contains(r#""implName": "rust""#);
    config_json.assert_contains(r#""sourceName": "api""#);
    config_json.assert_contains("api/code/**/*.rs");
    config_json.assert_contains("api/tests/**/*.rs");

    let validate = site
        .get("/_dodeca/coverage/validate.md?source=api&impl=rust")
        .await;
    validate.assert_ok();
    validate.assert_contains("Result: **failing**");
    validate.assert_contains("- Test impl references: `1`");

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
