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
    rule.assert_contains("## Implementation References");
    rule.assert_contains("## Verification References");
    rule.assert_contains("code/lib.rs");
}
