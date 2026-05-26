use super::*;

const CONFIG: &str = r#"
content content
output public

page-types {
  Decision @object{
    type @string
    supersedes @seq(@link(@Decision))
  }
  Note @object{
    type @string
  }
}
"#;

pub fn typed_frontmatter_link_to_same_type_passes() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", CONFIG),
            (
                "content/decision-a.md",
                r#"+++
title = "Decision A"

[extra]
type = "Decision"
supersedes = []
+++

# Decision A
"#,
            ),
            (
                "content/decision-b.md",
                r#"+++
title = "Decision B"

[extra]
type = "Decision"
supersedes = ["decision-a"]
+++

# Decision B
"#,
            ),
        ],
    );

    site.get("/decision-b/").assert_ok();
}

pub fn typed_frontmatter_missing_link_reports_error() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", CONFIG),
            (
                "content/decision-b.md",
                r#"+++
title = "Decision B"

[extra]
type = "Decision"
supersedes = ["missing-decision"]
+++

# Decision B
"#,
            ),
        ],
    );

    let html = site.get("/decision-b/");
    html.assert_ok();
    html.assert_contains("target 'missing-decision' not found for type Decision");
}

pub fn typed_frontmatter_wrong_target_type_reports_error() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (".config/dodeca.styx", CONFIG),
            (
                "content/note-a.md",
                r#"+++
title = "Note A"

[extra]
type = "Note"
+++

# Note A
"#,
            ),
            (
                "content/decision-b.md",
                r#"+++
title = "Decision B"

[extra]
type = "Decision"
supersedes = ["note-a"]
+++

# Decision B
"#,
            ),
        ],
    );

    let html = site.get("/decision-b/");
    html.assert_ok();
    html.assert_contains("target 'note-a' has wrong type; expected Decision");
}
