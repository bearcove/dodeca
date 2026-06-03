use super::*;

pub async fn rendered_markdown_route_returns_markdown() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                "content/source.md",
                r#"+++
title = "Source"
+++

# Source

See [[Target Page]] and [the source link](@/target-page.md).
"#,
            ),
            (
                "content/target-page.md",
                r#"+++
title = "Target Page"
+++

# Target Page
"#,
            ),
        ],
    );

    let markdown = site.get("/source.md").await;
    markdown.assert_ok();
    markdown.assert_content_type("text/markdown; charset=utf-8");
    markdown.assert_contains("title = \"Source\"");
    markdown.assert_contains("[Target Page](/target-page.md)");
    markdown.assert_contains("[the source link](/target-page.md)");
    markdown.assert_not_contains("[[Target Page]]");
}
