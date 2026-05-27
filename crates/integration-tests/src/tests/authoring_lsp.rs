use camino::{Utf8Path, Utf8PathBuf};
use dodeca::authoring_model::*;
use dodeca::authoring_model::{AuthoringInputPath, load_authoring_project};
use dodeca_authoring_lsp::authoring_lsp::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tower_lsp::lsp_types::*;

fn temp_dir(name: &str) -> Utf8PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
        "dodeca-authoring-lsp-{name}-{}-{nonce}",
        std::process::id()
    )))
    .expect("utf8 temp path")
}

fn position_for(content: &str, needle: &str) -> Position {
    let byte = content.find(needle).expect("needle in content");
    let (line, column) = byte_to_line_column(content, byte);
    Position {
        line: line - 1,
        character: column - 1,
    }
}

fn position_for_nth(content: &str, needle: &str, occurrence: usize) -> Position {
    let mut search_start = 0;
    for _ in 0..occurrence {
        let relative = content[search_start..]
            .find(needle)
            .expect("needle occurrence in content");
        search_start += relative + needle.len();
    }
    let relative = content[search_start..]
        .find(needle)
        .expect("needle occurrence in content");
    let byte = search_start + relative;
    let (line, column) = byte_to_line_column(content, byte);
    Position {
        line: line - 1,
        character: column - 1,
    }
}

fn range_for(content: &str, needle: &str) -> Range {
    let start_byte = content.find(needle).expect("needle in content");
    let end_byte = start_byte + needle.len();
    byte_range_to_lsp_range(content, start_byte, end_byte)
}

#[allow(deprecated)]
fn initialize_params_for_workspace(project_dir: &Utf8Path) -> InitializeParams {
    let uri = Url::from_directory_path(project_dir.as_std_path()).expect("workspace uri");
    InitializeParams {
        process_id: None,
        root_path: None,
        root_uri: None,
        initialization_options: None,
        capabilities: ClientCapabilities::default(),
        trace: None,
        workspace_folders: Some(vec![WorkspaceFolder {
            uri,
            name: "dodeca-site".to_string(),
        }]),
        client_info: None,
        locale: None,
    }
}

#[allow(deprecated)]
fn empty_initialize_params() -> InitializeParams {
    InitializeParams {
        process_id: None,
        root_path: None,
        root_uri: None,
        initialization_options: None,
        capabilities: ClientCapabilities::default(),
        trace: None,
        workspace_folders: None,
        client_info: None,
        locale: None,
    }
}

fn default_startup_args() -> LspStartupArgs {
    LspStartupArgs {
        content: None,
        output: None,
    }
}

#[test]
fn maps_missing_route_to_new_page_source_file() {
    assert_eq!(source_file_for_new_route("/ij"), Some("ij.md".to_string()));
    assert_eq!(
        source_file_for_new_route("/ops/deploy"),
        Some("ops/deploy.md".to_string())
    );
    assert_eq!(source_file_for_new_route("/"), None);
    assert_eq!(source_file_for_new_route("/bad:route"), None);
}

#[test]
fn creates_pages_with_frontmatter_only() {
    assert_eq!(
        page_frontmatter("New \"Page\""),
        "+++\ntitle = \"New \\\"Page\\\"\"\n+++\n"
    );
}

#[test]
fn code_action_creates_frontmatter_without_duplicate_markup_title() {
    let uri = Url::from_file_path("/tmp/dodeca/content/guide.md").expect("source uri");
    let page = AuthoringPage {
        kind: AuthoringPageKind::Page,
        route: "/guide".to_string(),
        source_file: "guide.md".to_string(),
        title: "Guide".to_string(),
        description: None,
        template: "page.html".to_string(),
        output_path: "guide/index.html".to_string(),
        headings: Vec::new(),
        heading_ids: Vec::new(),
        link_base_route: "/".to_string(),
    };
    let action = create_frontmatter_code_action(
        &uri,
        &page,
        "# Guide\n\nBody\n",
        Range {
            start: Position::new(0, 0),
            end: Position::new(0, 0),
        },
    )
    .expect("frontmatter action");
    let CodeActionOrCommand::CodeAction(action) = action else {
        panic!("expected code action");
    };
    let edit = action.edit.expect("workspace edit");
    let edits = edit
        .changes
        .expect("changes")
        .remove(&uri)
        .expect("source edits");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "+++\ntitle = \"Guide\"\n+++\n\n");
    assert!(!edits[0].new_text.contains("# Guide"));
}

#[test]
fn recognizes_frontmatter_as_page_identity_range() {
    let content = "+++\ntitle = \"Guide\"\n+++\n\nBody\n";
    let range = frontmatter_lsp_range(content).expect("frontmatter range");

    assert!(range_contains_position(
        &range,
        position_for(content, "title")
    ));
    assert!(!range_contains_position(
        &range,
        position_for(content, "Body")
    ));
}

#[test]
fn validates_frontmatter_against_typed_fields() {
    let content = "+++\ntitl = \"Typo\"\ntitle = \"Guide\"\ntitle = \"Duplicate\"\nweight = \"heavy\"\ntemplate = 42\n[extra]\ncustom = true\n+++\n";
    let diagnostics = frontmatter_diagnostics_for_source("guide.md", "/guide", content);
    let messages = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert!(messages.contains(&"unknown Dodeca frontmatter field 'titl'"));
    assert!(messages.contains(&"duplicate Dodeca frontmatter field 'title'"));
    assert!(messages.contains(&"frontmatter field 'weight' expects an integer"));
    assert!(messages.contains(&"frontmatter field 'template' expects a string"));
    assert!(!messages.iter().any(|message| message.contains("custom")));
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind == AuthoringDiagnosticKind::Frontmatter)
    );
}

#[test]
fn completes_missing_frontmatter_fields_from_schema() {
    let content = "+++\ntitle = \"Guide\"\n\n+++\n";
    let context = frontmatter_completion_context(content, Position::new(2, 0)).expect("context");
    let items = completion_items_for_frontmatter("guide.md", &context);
    let labels = items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();

    assert!(!labels.contains(&"title"));
    assert!(labels.contains(&"description"));
    assert!(labels.contains(&"weight"));
    assert!(labels.contains(&"template"));
    assert!(labels.contains(&"[extra]"));
    assert!(items.iter().any(|item| {
        item.text_edit.as_ref().is_some_and(|edit| match edit {
            CompletionTextEdit::Edit(edit) => edit.new_text == "template = \"page.html\"",
            CompletionTextEdit::InsertAndReplace(_) => false,
        })
    }));
}

#[tokio::test]
async fn resolves_frontmatter_template_document_targets_from_authoring_model() {
    let dir = temp_dir("frontmatter-template-link");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    let static_dir = dir.join("static");
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::write(templates_dir.join("custom.html"), "{{ page.content }}")
        .expect("write template");
    std::fs::write(static_dir.join("logo.png"), "png").expect("write static asset");
    std::fs::write(data_dir.join("versions.toml"), "stable = \"1.0\"").expect("write data");

    let content = "+++\ntitle = \"Guide\"\ntemplate = \"custom.html\"\nasset = \"/logo.png\"\ndata = \"versions.toml\"\n+++\n";
    std::fs::write(content_dir.join("guide.md"), content).expect("write page");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let world = AuthoringWorld::new(project).expect("authoring world");

    let targets = world.source_document_targets("guide.md");
    assert_eq!(targets.len(), 3);

    let template_target = targets
        .iter()
        .find(|target| target.kind == FrontmatterDocumentKind::Template)
        .expect("template target");
    assert_eq!(template_target.path, "custom.html");
    assert_eq!(
        template_target.target_path,
        templates_dir.join("custom.html")
    );
    assert!(range_contains_position(
        &template_target.source_range,
        position_for(content, "custom.html")
    ));

    let asset_target = targets
        .iter()
        .find(|target| target.kind == FrontmatterDocumentKind::StaticAsset)
        .expect("asset target");
    assert_eq!(asset_target.path, "/logo.png");
    assert_eq!(asset_target.target_path, static_dir.join("logo.png"));
    assert!(range_contains_position(
        &asset_target.source_range,
        position_for(content, "logo.png")
    ));

    let data_target = targets
        .iter()
        .find(|target| target.kind == FrontmatterDocumentKind::DataFile)
        .expect("data target");
    assert_eq!(data_target.path, "versions.toml");
    assert_eq!(data_target.target_path, data_dir.join("versions.toml"));
    assert!(range_contains_position(
        &data_target.source_range,
        position_for(content, "versions.toml")
    ));

    let target = world
        .source_document_target_at_position("guide.md", position_for(content, "custom.html"))
        .expect("target");
    assert_eq!(target.kind, FrontmatterDocumentKind::Template);
    assert_eq!(
        target.target_uri().expect("target uri"),
        Url::from_file_path(templates_dir.join("custom.html").as_std_path()).expect("expected uri")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn resolves_template_path_document_targets_from_authoring_model() {
    let dir = temp_dir("template-path-links");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    let source = "+++\ntitle = \"Uses Base\"\ntemplate = \"base.html\"\n+++\n\n# Uses Base\n";
    std::fs::write(content_dir.join("uses-base.md"), source).expect("write source");
    std::fs::write(
        templates_dir.join("base.ddc.html"),
        "{% block content %}{% endblock %}",
    )
    .expect("write base");
    std::fs::write(templates_dir.join("partial.html"), "<p>Partial</p>").expect("write partial");
    std::fs::write(
        templates_dir.join("macros.html"),
        "{% macro card(title) %}{{ title }}{% endmacro %}",
    )
    .expect("write macros");

    let child = "{% extends \"base.html\" %}\n{% include \"partial.html\" %}\n{% import \"macros.html\" as macros %}\n";
    std::fs::write(templates_dir.join("child.html"), child).expect("write child");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let world = AuthoringWorld::new(project).expect("authoring world");

    let targets = world.template_index.document_targets("child.html");
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0].kind, TemplateDocumentKind::Extends);
    assert_eq!(targets[0].path, "base.html");
    assert_eq!(targets[0].target_path, templates_dir.join("base.ddc.html"));
    assert_eq!(targets[1].kind, TemplateDocumentKind::Include);
    assert_eq!(targets[1].path, "partial.html");
    assert_eq!(targets[1].target_path, templates_dir.join("partial.html"));
    assert_eq!(targets[2].kind, TemplateDocumentKind::Import);
    assert_eq!(targets[2].path, "macros.html");
    assert_eq!(targets[2].target_path, templates_dir.join("macros.html"));
    let references = world
        .template_document_references(&content_dir, &targets[0].target_path)
        .expect("template document references");
    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&dir)
                    .expect("project relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "content/uses-base.md".to_string(),
            "templates/child.html".to_string(),
        ]
    );

    let target = world
        .template_index
        .document_target_at_position("child.html", position_for(child, "partial.html"))
        .expect("target");
    assert_eq!(target.kind, TemplateDocumentKind::Include);
    assert_eq!(
        target.target_uri().expect("target uri"),
        Url::from_file_path(templates_dir.join("partial.html").as_std_path())
            .expect("expected uri")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn resolves_template_definition_targets_from_authoring_model() {
    let dir = temp_dir("template-definitions");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let base = "{% block content %}Base{% endblock %}\n";
    let macros = "{% macro card(title) %}{{ title }}{% endmacro %}\n";
    let child = "{% extends \"base.html\" %}\n{% import \"macros.html\" as macros %}\n{% block content %}{{ macros::card(\"Hi\") }} {{ title | trim }}{% if title is string %}ok{% endif %}{% endblock %}\n";

    std::fs::write(templates_dir.join("base.html"), base).expect("write base");
    std::fs::write(templates_dir.join("macros.html"), macros).expect("write macros");
    std::fs::write(templates_dir.join("child.html"), child).expect("write child");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let targets =
        template_definition_targets(&project, "child.html", child).expect("definition targets");

    let block = targets
        .iter()
        .find(|target| target.kind == TemplateDefinitionKind::Block && target.name == "content")
        .expect("block target");
    assert!(range_contains_position(
        &block.source_range,
        position_for(child, "content")
    ));
    assert_eq!(block.target_path, templates_dir.join("base.html"));
    assert!(range_contains_position(
        &block.target_range,
        position_for(base, "content")
    ));

    let macro_target = targets
        .iter()
        .find(|target| {
            target.kind == TemplateDefinitionKind::Macro && target.name == "macros::card"
        })
        .expect("macro target");
    assert!(range_contains_position(
        &macro_target.source_range,
        position_for(child, "card")
    ));
    assert_eq!(macro_target.target_path, templates_dir.join("macros.html"));
    assert!(range_contains_position(
        &macro_target.target_range,
        position_for(macros, "card")
    ));

    let filter = targets
        .iter()
        .find(|target| target.kind == TemplateDefinitionKind::Filter && target.name == "trim")
        .expect("filter target");
    assert!(range_contains_position(
        &filter.source_range,
        position_for(child, "trim")
    ));
    assert!(filter.target_path.ends_with("gingembre/src/eval.rs"));
    assert!(
        filter
            .hover_markdown()
            .contains("Removes leading and trailing whitespace")
    );

    let test = targets
        .iter()
        .find(|target| target.kind == TemplateDefinitionKind::Test && target.name == "string")
        .expect("test target");
    assert!(range_contains_position(
        &test.source_range,
        position_for(child, "string")
    ));
    assert!(test.target_path.ends_with("gingembre/src/eval.rs"));
    assert!(test.hover_markdown().contains("value is a string"));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn finds_imported_template_macro_references() {
    let dir = temp_dir("template-macro-references");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let macros = "{% macro card(title) %}{{ title }}{% endmacro %}\n";
    let page = "{% import \"macros.html\" as ui %}\n{{ ui::card(\"Hi\") }}\n";
    let other = "{% import \"macros.html\" as kit %}\n{% set rendered = kit::card(\"Yo\") %}\n";
    std::fs::write(templates_dir.join("macros.html"), macros).expect("write macros");
    std::fs::write(templates_dir.join("page.html"), page).expect("write page");
    std::fs::write(templates_dir.join("other.html"), other).expect("write other");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");

    let index = TemplateAuthoringIndex::new(&project);
    let definition_query = index
        .macro_reference_query("macros.html", position_for(macros, "card"))
        .expect("definition query");
    assert_eq!(definition_query.target_template_file, "macros.html");
    assert_eq!(definition_query.macro_name, "card");
    assert!(range_contains_position(
        &definition_query.source_range,
        position_for(macros, "card")
    ));

    let call_query = index
        .macro_reference_query("page.html", position_for(page, "card"))
        .expect("call query");
    assert_eq!(call_query.target_template_file, "macros.html");
    assert_eq!(call_query.macro_name, "card");
    assert!(range_contains_position(
        &call_query.source_range,
        position_for(page, "card")
    ));
    let definition_target = index
        .macro_definition_target("macros.html", "card")
        .expect("macro definition target");
    assert_eq!(definition_target.path, templates_dir.join("macros.html"));
    assert!(range_contains_position(
        &definition_target.range,
        position_for(macros, "card")
    ));

    let references = template_macro_references(&index, "macros.html", "card").expect("references");
    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&templates_dir)
                    .expect("template relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "macros.html".to_string(),
            "other.html".to_string(),
            "page.html".to_string(),
        ]
    );
    for (content, index) in [(macros, 0), (other, 0), (page, 0)] {
        assert!(references.iter().any(|location| range_contains_position(
            &location.range,
            position_for_nth(content, "card", index)
        )));
    }

    let edit = template_macro_rename_workspace_edit(&index, "macros.html", "card", "panel")
        .expect("rename edit")
        .expect("rename edit");
    let changes = edit.changes.expect("changes");
    let mut changed_paths = changes
        .iter()
        .map(|(uri, edits)| {
            let path = Utf8PathBuf::from_path_buf(uri.to_file_path().expect("file uri"))
                .expect("utf8 path")
                .strip_prefix(&templates_dir)
                .expect("template relative")
                .to_string();
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].new_text, "panel");
            path
        })
        .collect::<Vec<_>>();
    changed_paths.sort();
    assert_eq!(
        changed_paths,
        vec![
            "macros.html".to_string(),
            "other.html".to_string(),
            "page.html".to_string(),
        ]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn resolves_template_block_references() {
    let dir = temp_dir("template-block-references");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let base = "{% block breadcrumbs %}Base{% endblock %}\n";
    let child = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
    let sibling = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Sibling{% endblock %}\n";
    std::fs::write(templates_dir.join("base.html"), base).expect("write base");
    std::fs::write(templates_dir.join("child.html"), child).expect("write child");
    std::fs::write(templates_dir.join("sibling.html"), sibling).expect("write sibling");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let index = TemplateAuthoringIndex::new(&project);
    let occurrence = index
        .block_occurrence_at_position("child.html", position_for(child, "breadcrumbs"))
        .expect("block occurrence");
    assert_eq!(occurrence.name, "breadcrumbs");

    let target = index
        .block_definition_target("child.html", &occurrence)
        .expect("parent block target");
    assert_eq!(target.target_path, templates_dir.join("base.html"));
    assert!(range_contains_position(
        &target.target_range,
        position_for(base, "breadcrumbs")
    ));

    let hover = template_block_hover_markdown(&index, "child.html", &occurrence);
    assert!(hover.contains("Overrides"));
    assert!(hover.contains("3 matching block declaration"));

    let references =
        template_block_references(&index, "child.html", "breadcrumbs").expect("block references");
    assert_eq!(references.len(), 3);
    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&templates_dir)
                    .expect("template relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "base.html".to_string(),
            "child.html".to_string(),
            "sibling.html".to_string(),
        ]
    );

    let edit = template_block_rename_workspace_edit(&index, "child.html", "breadcrumbs", "trail")
        .expect("rename edit")
        .expect("rename edit");
    let changes = edit.changes.expect("changes");
    let mut changed_paths = changes
        .iter()
        .map(|(uri, edits)| {
            let path = Utf8PathBuf::from_path_buf(uri.to_file_path().expect("file uri"))
                .expect("utf8 path")
                .strip_prefix(&templates_dir)
                .expect("template relative")
                .to_string();
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].new_text, "trail");
            path
        })
        .collect::<Vec<_>>();
    changed_paths.sort();
    assert_eq!(
        changed_paths,
        vec![
            "base.html".to_string(),
            "child.html".to_string(),
            "sibling.html".to_string(),
        ]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn resolves_template_block_references_by_inheritance_tree() {
    let dir = temp_dir("template-block-inheritance-references");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let base = "{% block breadcrumbs %}Base{% endblock %}\n";
    let child = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
    let grandchild =
        "{% extends \"child.html\" %}\n{% block breadcrumbs %}Grandchild{% endblock %}\n";
    let other_base = "{% block breadcrumbs %}Other base{% endblock %}\n";
    let other_child =
        "{% extends \"other-base.html\" %}\n{% block breadcrumbs %}Other child{% endblock %}\n";
    std::fs::write(templates_dir.join("base.html"), base).expect("write base");
    std::fs::write(templates_dir.join("child.html"), child).expect("write child");
    std::fs::write(templates_dir.join("grandchild.html"), grandchild).expect("write grandchild");
    std::fs::write(templates_dir.join("other-base.html"), other_base).expect("write other base");
    std::fs::write(templates_dir.join("other-child.html"), other_child).expect("write other child");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let index = TemplateAuthoringIndex::new(&project);

    let child_occurrence = index
        .block_occurrence_at_position("child.html", position_for(child, "breadcrumbs"))
        .expect("child block occurrence");
    let target = index
        .block_definition_target("child.html", &child_occurrence)
        .expect("child parent block");
    assert_eq!(target.target_path, templates_dir.join("base.html"));

    let references =
        template_block_references(&index, "child.html", "breadcrumbs").expect("references");
    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&templates_dir)
                    .expect("template relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "base.html".to_string(),
            "child.html".to_string(),
            "grandchild.html".to_string(),
        ]
    );

    let other_index = TemplateAuthoringIndex::new(&project);
    let other_references =
        template_block_references(&other_index, "other-child.html", "breadcrumbs")
            .expect("other references");
    assert_eq!(
        other_references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&templates_dir)
                    .expect("template relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "other-base.html".to_string(),
            "other-child.html".to_string(),
        ]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn template_block_index_uses_current_template_content() {
    let dir = temp_dir("template-block-live-content");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let base = "{% block breadcrumbs %}Base{% endblock %}\n";
    let other_base = "{% block breadcrumbs %}Other base{% endblock %}\n";
    let disk_child = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
    let live_child =
        "{% extends \"other-base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
    std::fs::write(templates_dir.join("base.html"), base).expect("write base");
    std::fs::write(templates_dir.join("other-base.html"), other_base).expect("write other base");
    std::fs::write(templates_dir.join("child.html"), disk_child).expect("write child");

    let project = load_authoring_project(
        &content_dir,
        &[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Template("child.html".to_string()),
            content: live_child.to_string(),
        }],
    )
    .await
    .expect("load project");
    let index = TemplateAuthoringIndex::new(&project);
    let references =
        template_block_references(&index, "child.html", "breadcrumbs").expect("references");

    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&templates_dir)
                    .expect("template relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec!["child.html".to_string(), "other-base.html".to_string()]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn exposes_template_document_symbols() {
    let dir = temp_dir("template-symbols");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let template =
        "{% block content %}{% macro card(title) %}{{ title }}{% endmacro %}{% endblock %}\n";
    std::fs::write(templates_dir.join("page.html"), template).expect("write template");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");

    let symbols =
        template_document_symbols(&project, "page.html", template).expect("document symbols");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "content");
    assert_eq!(symbols[0].kind, SymbolKind::MODULE);
    let children = symbols[0].children.as_ref().expect("block children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "card");
    assert_eq!(children[0].kind, SymbolKind::FUNCTION);

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn uses_template_semantic_index_for_editor_features() {
    let dir = temp_dir("template-semantic-editor-features");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let template = "{% set local_route = \"/\" %}\n{{ local_route }}\n{% for item in section.pages %}\n{{ item.path | path_parent }}\n{% endfor %}\n{% for child in section.subsections %}\n{{ child.pages }}\n{% endfor %}\n{% macro label(title) %}{{ title }}{% endmacro %}\n";
    std::fs::write(templates_dir.join("page.html"), template).expect("write template");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let template_index = TemplateAuthoringIndex::new(&project);
    let semantic_index = template_index
        .templates
        .get("page.html")
        .and_then(|template| template.semantic.as_ref())
        .expect("template semantics");

    let hover =
        template_semantic_hover(&template_index, "page.html", position_for(template, "path"))
            .expect("field hover");
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markdown hover");
    };
    assert!(markup.value.contains("Route path"));
    assert!(markup.value.contains("Site-relative route"));

    let filter_hover = template_semantic_hover(
        &template_index,
        "page.html",
        position_for(template, "path_parent"),
    )
    .expect("filter hover");
    let HoverContents::Markup(markup) = filter_hover.contents else {
        panic!("expected filter markdown hover");
    };
    assert!(markup.value.contains("Gingembre filter"));
    assert!(markup.value.contains("Returns the parent path"));

    let section_field_hover = template_semantic_hover(
        &template_index,
        "page.html",
        position_for_nth(template, "pages", 1),
    )
    .expect("section field hover through loop binding");
    let HoverContents::Markup(markup) = section_field_hover.contents else {
        panic!("expected section field markdown hover");
    };
    assert!(markup.value.contains("Section pages"));
    assert!(markup.value.contains("nearest parent section"));

    let local_hover = template_semantic_hover(
        &template_index,
        "page.html",
        position_for_nth(template, "local_route", 1),
    )
    .expect("local variable hover");
    let HoverContents::Markup(markup) = local_hover.contents else {
        panic!("expected local variable markdown hover");
    };
    assert!(markup.value.contains("Read reference"));
    assert!(markup.value.contains("1 read reference"));
    assert!(markup.value.contains("0 write reference"));

    let definition = template_index
        .semantic_definition("page.html", position_for_nth(template, "local_route", 1))
        .expect("local binding definition");
    assert!(range_contains_position(
        &definition.range,
        position_for(template, "local_route")
    ));

    let references = template_index
        .semantic_references("page.html", position_for_nth(template, "local_route", 1));
    assert_eq!(references.len(), 2);
    assert!(references.iter().any(|location| range_contains_position(
        &location.range,
        position_for(template, "local_route")
    )));
    assert!(references.iter().any(|location| range_contains_position(
        &location.range,
        position_for_nth(template, "local_route", 1)
    )));

    let prepared = template_semantic_prepare_rename(
        &template_index,
        "page.html",
        position_for_nth(template, "local_route", 1),
    )
    .expect("prepare rename");
    let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = prepared else {
        panic!("expected range with placeholder");
    };
    assert_eq!(placeholder, "local_route");

    let uri =
        Url::from_file_path(templates_dir.join("page.html").as_std_path()).expect("template uri");
    let edit = template_semantic_rename_workspace_edit(
        &template_index,
        "page.html",
        position_for_nth(template, "local_route", 1),
        "route_path",
    )
    .expect("rename edit")
    .expect("rename edit");
    let mut changes = edit.changes.expect("rename changes");
    let edits = changes.remove(&uri).expect("template edits");
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit.new_text == "route_path"));

    let token_types = template_semantic_tokens(semantic_index, template)
        .into_iter()
        .map(|token| token.token_type)
        .collect::<Vec<_>>();
    assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_VARIABLE));
    assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_PARAMETER));
    assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_PROPERTY));
    assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_MACRO));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn repeated_template_set_bindings_share_references() {
    let dir = temp_dir("template-repeated-set-references");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let template = "{% set current_path = \"/\" %}\n{% if section is defined %}{% set current_path = section.path %}{% endif %}\n{% if page is defined %}{% set current_path = page.path %}{% endif %}\n<a class=\"{% if current_path is eq(\"/\") %}is-active{% endif %}\">{{ current_path }}</a>\n";
    std::fs::write(templates_dir.join("base.html"), template).expect("write template");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let template_index = TemplateAuthoringIndex::new(&project);

    let first_definition = template_index
        .semantic_definition("base.html", position_for(template, "current_path"))
        .expect("first definition");
    let second_definition = template_index
        .semantic_definition("base.html", position_for_nth(template, "current_path", 1))
        .expect("second definition");
    assert_eq!(first_definition.range, second_definition.range);
    assert!(range_contains_position(
        &first_definition.range,
        position_for(template, "current_path")
    ));

    let first_references =
        template_index.semantic_references("base.html", position_for(template, "current_path"));
    let second_references = template_index
        .semantic_references("base.html", position_for_nth(template, "current_path", 1));
    assert_eq!(first_references, second_references);
    assert_eq!(first_references.len(), 5);
    for index in 0..5 {
        assert!(
            first_references
                .iter()
                .any(|location| range_contains_position(
                    &location.range,
                    position_for_nth(template, "current_path", index)
                ))
        );
    }

    let uri = Url::from_file_path(templates_dir.join("base.html").as_std_path()).expect("file uri");
    let edit = template_semantic_rename_workspace_edit(
        &template_index,
        "base.html",
        position_for(template, "current_path"),
        "active_path",
    )
    .expect("rename edit")
    .expect("rename edit");
    let edits = edit
        .changes
        .expect("changes")
        .remove(&uri)
        .expect("template edits");
    assert_eq!(edits.len(), 5);

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn reports_template_diagnostics_from_authoring_model() {
    let dir = temp_dir("template-diagnostics");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    std::fs::write(
        templates_dir.join("base.html"),
        "{% block title %}Title{% endblock %}\n",
    )
    .expect("write base");
    std::fs::write(
        templates_dir.join("macros.html"),
        "{% macro card(title) %}{{ title }}{% endmacro %}\n",
    )
    .expect("write macros");
    let child = "{% extends \"base.html\" %}\n{% include \"missing.html\" %}\n{% import \"macros.html\" as macros %}\n{% block content %}{{ macros::missing(\"Hi\") | nope }}{% if title is frobnicate %}ok{% endif %}{% endblock %}\n";
    std::fs::write(templates_dir.join("child.html"), child).expect("write child");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let diagnostics = diagnostics_for_template(&project, "child.html", child);
    let kinds = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind)
        .collect::<Vec<_>>();

    assert!(kinds.contains(&AuthoringDiagnosticKind::MissingTemplate));
    assert!(kinds.contains(&AuthoringDiagnosticKind::MissingBlock));
    assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownMacro));
    assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownFilter));
    assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownTest));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::MissingTemplate
            && diagnostic.target == "missing.html"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::MissingBlock && diagnostic.target == "content"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::UnknownMacro && diagnostic.target == "missing"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::UnknownFilter && diagnostic.target == "nope"
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::UnknownTest && diagnostic.target == "frobnicate"
    }));
    let lsp_diagnostics = diagnostics
        .iter()
        .map(authoring_diagnostic_to_lsp)
        .collect::<Vec<_>>();
    let missing_template = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.kind == AuthoringDiagnosticKind::MissingTemplate)
        .expect("missing template diagnostic");
    let actions = missing_template_code_actions(&content_dir, missing_template, &lsp_diagnostics)
        .expect("missing template code actions");
    let action = actions
        .iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => Some(action),
            CodeActionOrCommand::Command(_) => None,
        })
        .expect("code action");
    assert_eq!(action.title, "Create template 'missing.html'");
    let edit = action.edit.as_ref().expect("workspace edit");
    let operations = match edit.document_changes.as_ref().expect("document changes") {
        DocumentChanges::Operations(operations) => operations,
        DocumentChanges::Edits(_) => panic!("expected document change operations"),
    };
    assert!(matches!(
        &operations[..],
        [DocumentChangeOperation::Op(ResourceOp::Create(CreateFile { uri, .. }))]
            if *uri == template_file_uri(&content_dir, "missing.html").expect("template uri")
    ));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn completes_template_context_filters_tests_data_and_macros() {
    let dir = temp_dir("template-completions");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(data_dir.join("versions.toml"), "dodeca = \"0.1.0\"\n").expect("write data");
    std::fs::write(
        templates_dir.join("macros.html"),
        "{% macro card(title) %}{{ title }}{% endmacro %}\n",
    )
    .expect("write macros");

    let template = "{% import \"macros.html\" as macros %}\n{% set local_route = \"/\" %}\n{% set current_page = page %}\n{{ loc }}\n{{ pa }}\n{{ page.ti }}\n{{ current_page.pa }}\n{% for item in section.pages %}\n{{ item.pa }}\n{% endfor %}\n{{ data.ver }}\n{{ title | tr }}\n{% if title is str %}ok{% endif %}\n{{ macros::card() }}\n";
    std::fs::write(templates_dir.join("page.html"), template).expect("write template");
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");

    let labels = |position| {
        template_completion_items(&project, "page.html", template, position)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
    };

    let root_items = template_completion_items(
        &project,
        "page.html",
        template,
        position_for(template, "pa"),
    );
    let page_item = root_items
        .iter()
        .find(|item| item.label == "page")
        .expect("page completion item");
    assert_eq!(page_item.detail.as_deref(), Some("Current page"));
    let Some(Documentation::MarkupContent(documentation)) = &page_item.documentation else {
        panic!("expected page completion docs");
    };
    assert!(documentation.value.contains("currently being rendered"));

    assert!(
        labels(position_for(template, "loc")).contains(&"local_route".to_string()),
        "root completion should include live Gingembre local symbols"
    );
    assert!(labels(position_for(template, "pa")).contains(&"page".to_string()));
    assert!(
        labels(position_for(template, "ti")).contains(&"title".to_string()),
        "page field completion should include title"
    );
    let current_page_field_byte = template
        .find("current_page.pa")
        .expect("current page alias field")
        + "current_page.".len();
    let (line, column) = byte_to_line_column(template, current_page_field_byte);
    assert!(
        labels(Position::new(line - 1, column - 1)).contains(&"path".to_string()),
        "set aliases should complete fields from their source value"
    );
    let loop_item_field_byte = template.find("item.pa").expect("loop item field") + "item.".len();
    let (line, column) = byte_to_line_column(template, loop_item_field_byte);
    assert!(
        labels(Position::new(line - 1, column - 1)).contains(&"permalink".to_string()),
        "loop bindings over section.pages should complete page fields"
    );
    assert!(
        labels(position_for(template, "ver")).contains(&"versions".to_string()),
        "data completion should include data file stem"
    );
    assert!(
        labels(position_for(template, "tr")).contains(&"trim".to_string()),
        "filter completion should include trim"
    );
    let filter_items = template_completion_items(
        &project,
        "page.html",
        template,
        position_for(template, "tr"),
    );
    let trim_item = filter_items
        .iter()
        .find(|item| item.label == "trim")
        .expect("trim completion item");
    let Some(Documentation::MarkupContent(documentation)) = &trim_item.documentation else {
        panic!("expected trim completion docs");
    };
    assert!(
        documentation
            .value
            .contains("Removes leading and trailing whitespace")
    );
    assert!(
        labels(position_for(template, "str")).contains(&"string".to_string()),
        "test completion should include string"
    );
    let macro_byte = template.find("macros::").expect("macro call") + "macros::".len();
    let (line, column) = byte_to_line_column(template, macro_byte);
    assert!(
        labels(Position::new(line - 1, column - 1)).contains(&"card".to_string()),
        "macro completion should include imported macro"
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn resolves_authoring_dirs_from_lsp_workspace_folder() {
    let dir = temp_dir("initialize-root");
    std::fs::create_dir_all(dir.join(".config")).expect("create config dir");
    std::fs::create_dir_all(dir.join("content")).expect("create content dir");
    std::fs::write(
        dir.join(".config/dodeca.styx"),
        "content content\noutput public\n",
    )
    .expect("write config");

    let params = initialize_params_for_workspace(&dir);
    let dirs = resolve_initial_authoring_dirs(&default_startup_args(), &params)
        .expect("resolve dirs")
        .expect("workspace config");

    assert_eq!(dirs.content_dir, dir.join("content"));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn initializes_without_lsp_workspace() {
    let dirs = resolve_initial_authoring_dirs(&default_startup_args(), &empty_initialize_params())
        .expect("initialize without workspace");

    assert!(dirs.is_none());
}

#[test]
fn resolves_authoring_dirs_from_lsp_workspace_descendant() {
    let project = temp_dir("initialize-descendant");
    let content_dir = project.join("content");
    std::fs::create_dir_all(project.join(".config")).expect("create config dir");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::write(
        project.join(".config/dodeca.styx"),
        "content content\noutput public\n",
    )
    .expect("write config");

    let params = initialize_params_for_workspace(&content_dir);
    let dirs = resolve_initial_authoring_dirs(&default_startup_args(), &params)
        .expect("resolve dirs")
        .expect("workspace config");

    assert_eq!(dirs.content_dir, content_dir);

    std::fs::remove_dir_all(&project).expect("remove temp dir");
}

#[test]
fn resolves_authoring_dirs_from_document_ancestor_config() {
    let workspace = temp_dir("document-root");
    let project = workspace.join("kb.vixen.rs");
    std::fs::create_dir_all(project.join(".config")).expect("create config dir");
    std::fs::create_dir_all(project.join("content/ops")).expect("create content dir");
    std::fs::write(
        project.join(".config/dodeca.styx"),
        "content content\noutput public\n",
    )
    .expect("write config");
    let document = project.join("content/ops/deploy.md");
    std::fs::write(&document, "# Deploy\n").expect("write document");
    let uri = Url::from_file_path(document.as_std_path()).expect("document uri");

    let dirs = resolve_authoring_dirs_for_document(&default_startup_args(), &uri)
        .expect("resolve document dirs");

    assert_eq!(dirs.content_dir, project.join("content"));

    std::fs::remove_dir_all(&workspace).expect("remove temp dir");
}

#[tokio::test]
async fn lists_pages_and_sections_from_content_dir() {
    let dir = temp_dir("list-pages");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create dirs");
    std::fs::write(
        content_dir.join("_index.md"),
        "+++\ntitle = \"Knowledge Base\"\n+++\n\n# Home\n",
    )
    .expect("write root");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n## Details\n",
    )
    .expect("write page");

    let pages = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project")
        .pages;

    assert_eq!(
        pages,
        vec![
            AuthoringPage {
                kind: AuthoringPageKind::Section,
                route: "/".to_string(),
                source_file: "_index.md".to_string(),
                title: "Knowledge Base".to_string(),
                description: None,
                template: "index.html".to_string(),
                output_path: "index.html".to_string(),
                headings: vec![AuthoringHeading {
                    id: "home".to_string(),
                    title: "Home".to_string(),
                    level: 1,
                }],
                heading_ids: vec!["home".to_string()],
                link_base_route: "/".to_string(),
            },
            AuthoringPage {
                kind: AuthoringPageKind::Page,
                route: "/guide/intro".to_string(),
                source_file: "guide/intro.md".to_string(),
                title: "Intro".to_string(),
                description: None,
                template: "page.html".to_string(),
                output_path: "guide/intro/index.html".to_string(),
                headings: vec![
                    AuthoringHeading {
                        id: "intro".to_string(),
                        title: "Intro".to_string(),
                        level: 1,
                    },
                    AuthoringHeading {
                        id: "intro--details".to_string(),
                        title: "Details".to_string(),
                        level: 2,
                    },
                ],
                heading_ids: vec!["intro".to_string(), "intro--details".to_string()],
                link_base_route: "/".to_string(),
            },
        ]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn loads_authoring_diagnostics_from_content_dir() {
    let dir = temp_dir("diagnostics-command");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::write(
        content_dir.join("_index.md"),
        "+++\ntitle = \"Home\"\n+++\n\n[Missing](/missing)\n",
    )
    .expect("write root");

    let diagnostics = authoring_diagnostics_for_content_dir(&content_dir)
        .await
        .expect("authoring diagnostics");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == AuthoringDiagnosticKind::Route
            && diagnostic.target == "/missing"
            && diagnostic.resolved_route.as_deref() == Some("/missing")
    }));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn reports_missing_routes_anchors_sources_and_static_assets() {
    let dir = temp_dir("diagnostics");
    let content_dir = dir.join("content");
    let static_dir = dir.join("static");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(content_dir.join("guide/intro.md"), "# Intro\n").expect("write target");
    let source = "\
# Source

[ok](/guide/intro#intro)
[missing route](/missing)
[missing anchor](/guide/intro#nope)
[missing source](@/guide/missing.md)
![missing image](/missing.png)
![ok image](/logo.png)
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let source_page = project
        .page_for_source_file("guide/source.md")
        .expect("source page");
    let diagnostics = diagnostics_for_page(&project, source_page, source);
    let kinds = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind)
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        vec![
            AuthoringDiagnosticKind::Route,
            AuthoringDiagnosticKind::Anchor,
            AuthoringDiagnosticKind::Source,
            AuthoringDiagnosticKind::StaticAsset,
        ]
    );
    assert_eq!(diagnostics[0].source_file, "guide/source.md");
    assert_eq!(diagnostics[0].resolved_route.as_deref(), Some("/missing"));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn reports_site_graph_diagnostics() {
    let dir = temp_dir("site-graph-diagnostics");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(content_dir.join("empty")).expect("create empty section");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(
        templates_dir.join("section.html"),
        "<nav><a href=\"/rendered\">Rendered</a></nav>{{ section.content | safe }}",
    )
    .expect("write section template");
    std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
        .expect("write page template");
    std::fs::write(
            content_dir.join("_index.md"),
            "+++\ntitle = \"Home\"\ntemplate = \"section.html\"\n+++\n\n[linked](/linked)\n[same a](/same-a)\n[same b](/same-b)\n",
        )
        .expect("write root");
    std::fs::write(
        content_dir.join("linked.md"),
        "+++\ntitle = \"Linked\"\n+++\n",
    )
    .expect("write linked");
    std::fs::write(
        content_dir.join("orphan.md"),
        "+++\ntitle = \"Orphan\"\n+++\n",
    )
    .expect("write orphan");
    std::fs::write(
        content_dir.join("rendered.md"),
        "+++\ntitle = \"Rendered\"\n+++\n",
    )
    .expect("write rendered-nav page");
    std::fs::write(
        content_dir.join("same-a.md"),
        "+++\ntitle = \"Same\"\n+++\n",
    )
    .expect("write same a");
    std::fs::write(
        content_dir.join("same-b.md"),
        "+++\ntitle = \"Same\"\n+++\n",
    )
    .expect("write same b");
    std::fs::write(
        content_dir.join("empty/_index.md"),
        "+++\ntitle = \"Empty\"\n+++\n",
    )
    .expect("write empty section");

    let mut project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    assert!(
        project
            .rendered_hrefs_by_route
            .get("/")
            .is_some_and(|hrefs| hrefs.iter().any(|href| href.href == "/rendered"))
    );
    let duplicate_route_source = project
        .pages
        .iter()
        .find(|page| page.source_file == "same-b.md")
        .map(|page| page.route.clone())
        .expect("same-b route");
    for page in &mut project.pages {
        if page.source_file == "same-b.md" {
            page.route = "/same-a".to_string();
        }
    }

    let diagnostics = site_graph_diagnostics(&project);
    let has = |kind, target: &str| {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == kind && diagnostic.target == target)
    };

    assert!(has(AuthoringDiagnosticKind::DuplicateTitle, "Same"));
    assert!(has(AuthoringDiagnosticKind::DuplicateRoute, "/same-a"));
    assert!(has(AuthoringDiagnosticKind::OrphanPage, "/orphan"));
    assert!(
        has(AuthoringDiagnosticKind::NoInboundLinks, "/empty/")
            || has(AuthoringDiagnosticKind::NoInboundLinks, "/empty")
    );
    assert!(!has(AuthoringDiagnosticKind::OrphanPage, "/linked"));
    assert!(!has(AuthoringDiagnosticKind::OrphanPage, "/rendered"));
    assert_ne!(duplicate_route_source, "/same-a");

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn exposes_route_graph_edges() {
    let dir = temp_dir("route-graph");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(
        templates_dir.join("section.html"),
        "<nav><a href=\"/rendered\">Rendered</a></nav>{{ section.content | safe }}",
    )
    .expect("write section template");
    std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
        .expect("write page template");
    std::fs::write(
        content_dir.join("_index.md"),
        "+++\ntitle = \"Home\"\ntemplate = \"section.html\"\n+++\n\n[guide](/guide)\n",
    )
    .expect("write root");
    std::fs::write(
        content_dir.join("guide.md"),
        "+++\ntitle = \"Guide\"\n+++\n\n[home](/)\n",
    )
    .expect("write guide");
    std::fs::write(
        content_dir.join("rendered.md"),
        "+++\ntitle = \"Rendered\"\n+++\n",
    )
    .expect("write rendered");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let graph = route_graph_for_project(&project);
    let home = graph
        .iter()
        .find(|node| node.route == "/")
        .expect("home node");
    let guide = graph
        .iter()
        .find(|node| node.route == "/guide")
        .expect("guide node");
    let rendered = graph
        .iter()
        .find(|node| node.route == "/rendered")
        .expect("rendered node");

    assert!(home.outgoing.iter().any(|edge| {
        edge.kind == RouteGraphEdgeKind::Markdown && edge.target_route == "/guide"
    }));
    assert!(home.outgoing.iter().any(|edge| {
        edge.kind == RouteGraphEdgeKind::RenderedHtml && edge.target_route == "/rendered"
    }));
    assert_eq!(guide.incoming.len(), 1);
    assert_eq!(guide.incoming[0].source_route, "/");
    assert_eq!(guide.outgoing.len(), 1);
    assert_eq!(guide.outgoing[0].target_route, "/");
    assert_eq!(rendered.incoming.len(), 1);
    assert_eq!(rendered.incoming[0].kind, RouteGraphEdgeKind::RenderedHtml);

    let json = route_graph_to_json(&graph);
    assert!(json.as_array().is_some_and(|nodes| nodes.len() == 3));
    assert!(
        json.to_string().contains("\"kind\":\"renderedHtml\""),
        "route graph JSON should preserve rendered edge provenance"
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn code_actions_fix_missing_anchors_from_authoring_headings() {
    let dir = temp_dir("anchor-actions");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    let target = "\
# Intro

## Details
";
    std::fs::write(content_dir.join("guide/intro.md"), target).expect("write target");
    let source = "\
# Source

[typo](/guide/intro#intro--detals)
[new](/guide/intro#intro--appendix)
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let source_page = project
        .page_for_source_file("guide/source.md")
        .expect("source page");
    let diagnostics = diagnostics_for_page(&project, source_page, source);
    let lsp_diagnostics = diagnostics
        .iter()
        .map(authoring_diagnostic_to_lsp)
        .collect::<Vec<_>>();
    let typo_diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.target.contains("intro--detals"))
        .expect("typo diagnostic");
    let actions = missing_anchor_code_actions(
        &content_dir,
        &project,
        source,
        typo_diagnostic,
        &lsp_diagnostics,
    );

    let titles = actions
        .iter()
        .filter_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => Some(action.title.as_str()),
            CodeActionOrCommand::Command(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(titles.contains(&"Change anchor to '#intro--details'"));
    assert!(titles.contains(&"Create heading for '#intro--detals'"));

    let source_uri = Url::from_file_path(content_dir.join("guide/source.md")).expect("source uri");
    let replacement = actions
        .iter()
        .filter_map(|action| match action {
            CodeActionOrCommand::CodeAction(action)
                if action.title == "Change anchor to '#intro--details'" =>
            {
                action.edit.as_ref()
            }
            _ => None,
        })
        .next()
        .expect("replacement edit");
    let replacement_edits = replacement
        .changes
        .as_ref()
        .and_then(|changes| changes.get(&source_uri))
        .expect("source replacement edits");
    assert_eq!(replacement_edits.len(), 1);
    assert_eq!(replacement_edits[0].new_text, "intro--details");

    let target_uri = Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri");
    let creation = actions
        .iter()
        .filter_map(|action| match action {
            CodeActionOrCommand::CodeAction(action)
                if action.title == "Create heading for '#intro--detals'" =>
            {
                action.edit.as_ref()
            }
            _ => None,
        })
        .next()
        .expect("creation edit");
    let creation_edits = creation
        .changes
        .as_ref()
        .and_then(|changes| changes.get(&target_uri))
        .expect("target creation edits");
    assert_eq!(creation_edits.len(), 1);
    assert_eq!(creation_edits[0].new_text, "\n\n## Detals\n");

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn code_action_extracts_selection_to_page_without_duplicate_title() {
    let dir = temp_dir("extract-page");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write guide");
    let source = "\
+++
title = \"Source\"
+++

# Source

Keep this.

## Extract Me

This moves.
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let uri = Url::from_file_path(content_dir.join("guide/source.md")).expect("source uri");
    let selection = range_for(source, "## Extract Me\n\nThis moves.");
    let plan = extract_page_plan(&content_dir, &project, &uri, source, selection)
        .expect("extract plan")
        .expect("extract action");

    assert_eq!(plan.source_file, "guide/source.md");
    assert_eq!(plan.new_source_file, "guide/extract-me.md");
    assert_eq!(plan.new_route, "/guide/extract-me");
    assert_eq!(plan.title, "Extract Me");
    assert_eq!(
        plan.new_content,
        "+++\ntitle = \"Extract Me\"\n+++\n\nThis moves.\n"
    );
    assert_eq!(plan.replacement, "[Extract Me](extract-me)");
    assert!(!plan.new_content.contains("## Extract Me"));

    let edit = workspace_edit_for_extract_page(&content_dir, &plan).expect("workspace edit");
    let operations = match edit.document_changes.expect("document changes") {
        DocumentChanges::Operations(operations) => operations,
        DocumentChanges::Edits(_) => panic!("expected document change operations"),
    };
    assert_eq!(operations.len(), 3);
    match &operations[0] {
        DocumentChangeOperation::Op(ResourceOp::Create(create)) => {
            assert_eq!(
                create.uri,
                Url::from_file_path(content_dir.join("guide/extract-me.md")).expect("created uri")
            );
        }
        _ => panic!("expected create file operation"),
    }

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn resolves_definition_locations_for_links_and_static_assets() {
    let dir = temp_dir("definition");
    let content_dir = dir.join("content");
    let static_dir = dir.join("static");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "# Intro\n\n## Details\n",
    )
    .expect("write target");
    std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write section");
    let source = "\
# Source

[route](/guide/intro#intro--details)
[source](@/guide/intro.md#intro)
[relative](intro#intro)
![logo](/logo.png)
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let dirs = AuthoringDirs {
        content_dir: content_dir.clone(),
    };
    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let page = project
        .page_for_source_file("guide/source.md")
        .expect("source page");

    let route_reference =
        reference_at_position(source, position_for(source, "/guide/intro#intro--details"))
            .expect("route reference");
    let route_location = definition_for_reference(&dirs, &project, page, &route_reference)
        .expect("route definition")
        .expect("route location");
    assert_eq!(
        route_location.uri,
        Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri")
    );
    assert_eq!(route_location.range.start.line, 2);

    let source_reference =
        reference_at_position(source, position_for(source, "@/guide/intro.md#intro"))
            .expect("source reference");
    let source_location = definition_for_reference(&dirs, &project, page, &source_reference)
        .expect("source definition")
        .expect("source location");
    assert_eq!(source_location.range.start.line, 0);

    let relative_reference = reference_at_position(source, position_for(source, "[relative]"))
        .expect("relative reference");
    let relative_location = definition_for_reference(&dirs, &project, page, &relative_reference)
        .expect("relative definition")
        .expect("relative location");
    assert_eq!(
        relative_location.uri,
        Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri")
    );

    let static_reference =
        reference_at_position(source, position_for(source, "/logo.png")).expect("image reference");
    let static_location = definition_for_reference(&dirs, &project, page, &static_reference)
        .expect("static definition")
        .expect("static location");
    assert_eq!(
        static_location.uri,
        Url::from_file_path(static_dir.join("logo.png")).expect("static uri")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn finds_references_to_page_routes_and_source_links() {
    let dir = temp_dir("references");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    let section_template = "<nav><a href=\"/target\">Target</a></nav>{{ section.content | safe }}";
    std::fs::write(templates_dir.join("section.html"), section_template)
        .expect("write section template");
    std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
        .expect("write page template");
    std::fs::write(
        content_dir.join("target.md"),
        "+++\ntitle = \"Target\"\n+++\n\n# Target\n",
    )
    .expect("write target");
    std::fs::write(
        content_dir.join("_index.md"),
        "\
+++
title = \"Home\"
template = \"section.html\"
+++

# Home

[route](/target)
[source](@/target.md)
[anchor](/target#target)
![asset](/target.png)
",
    )
    .expect("write root");
    std::fs::write(
        content_dir.join("nested/source.md"),
        "\
# Source

[relative](../target)
",
    )
    .expect("write nested source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let world = AuthoringWorld::new(project.clone()).expect("authoring world");
    let target_page = project
        .page_for_source_file("target.md")
        .expect("target page");
    let template_reference = world
        .template_index
        .route_reference_at_position("section.html", position_for(section_template, "/target"))
        .expect("template route reference");
    assert_eq!(template_reference.target_route, "/target");
    assert!(range_contains_position(
        &template_reference.source_range,
        position_for(section_template, "/target")
    ));

    let references = references_to_page(&content_dir, &project, target_page).expect("references");

    assert_eq!(references.len(), 5);
    assert_eq!(
        references
            .iter()
            .map(|location| {
                let path =
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path");
                path.strip_prefix(&dir)
                    .expect("project relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "content/_index.md".to_string(),
            "content/_index.md".to_string(),
            "content/_index.md".to_string(),
            "content/nested/source.md".to_string(),
            "templates/section.html".to_string(),
        ]
    );
    let template_reference = references
        .iter()
        .find(|location| {
            location.uri.to_file_path().is_ok_and(|path| {
                Utf8PathBuf::from_path_buf(path)
                    .ok()
                    .is_some_and(|path| path.ends_with("templates/section.html"))
            })
        })
        .expect("template rendered reference");
    assert!(range_contains_position(
        &template_reference.range,
        position_for(section_template, "/target")
    ));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn finds_references_to_exact_heading_fragments() {
    let dir = temp_dir("heading-references");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
    let target = "\
+++
title = \"Target\"
+++

# Target

## Details
";
    std::fs::write(content_dir.join("target.md"), target).expect("write target");
    std::fs::write(
        content_dir.join("_index.md"),
        "\
# Home

[page](/target)
[route heading](/target#target--details)
[source heading](@/target.md#target--details)
[wrong heading](/target#target)
",
    )
    .expect("write root");
    std::fs::write(
        content_dir.join("nested/source.md"),
        "\
# Source

[relative heading](../target#target--details)
",
    )
    .expect("write nested source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let target_page = project
        .page_for_source_file("target.md")
        .expect("target page");
    let heading_id = heading_id_at_position(target_page, target, position_for(target, "Details"))
        .expect("heading id");
    let references = references_to_heading(&content_dir, &project, target_page, &heading_id)
        .expect("heading references");

    assert_eq!(heading_id, "target--details");
    assert_eq!(references.len(), 3);
    assert_eq!(
        references
            .iter()
            .map(|location| {
                Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                    .expect("utf8 path")
                    .strip_prefix(&content_dir)
                    .expect("content relative")
                    .to_string()
            })
            .collect::<Vec<_>>(),
        vec![
            "_index.md".to_string(),
            "_index.md".to_string(),
            "nested/source.md".to_string(),
        ]
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn renames_heading_and_exact_fragment_links() {
    let dir = temp_dir("heading-rename");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
    let target = "\
+++
title = \"Target\"
+++

# Target

## Details
";
    std::fs::write(content_dir.join("target.md"), target).expect("write target");
    let index = "\
# Home

[page](/target)
[route heading](/target#target--details)
[source heading](@/target.md#target--details)
[wrong heading](/target#target)
";
    std::fs::write(content_dir.join("_index.md"), index).expect("write root");
    let nested = "\
# Source

[relative heading](../target#target--details)
";
    std::fs::write(content_dir.join("nested/source.md"), nested).expect("write nested source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let target_page = project
        .page_for_source_file("target.md")
        .expect("target page");
    let target_position = position_for(target, "Details");
    let target = heading_rename_target_at_position(target_page, target, target_position)
        .expect("rename target");
    assert_eq!(target.heading_id, "target--details");
    assert_eq!(target.title, "Details");

    let edit = rename_heading_workspace_edit(
        &content_dir,
        &project,
        target_page,
        "target.md",
        project
            .source_contents
            .get("target.md")
            .expect("target content"),
        &target,
        "Deep Details",
    )
    .expect("rename edit")
    .expect("workspace edit");
    let changes = edit.changes.expect("workspace changes");

    let target_uri = Url::from_file_path(content_dir.join("target.md")).expect("target uri");
    let index_uri = Url::from_file_path(content_dir.join("_index.md")).expect("index uri");
    let nested_uri = Url::from_file_path(content_dir.join("nested/source.md")).expect("nested uri");

    let target_edits = changes.get(&target_uri).expect("target edits");
    assert_eq!(target_edits.len(), 1);
    assert_eq!(target_edits[0].new_text, "Deep Details");
    assert_eq!(target_edits[0].range, target.title_range);

    let index_edits = changes.get(&index_uri).expect("index edits");
    assert_eq!(
        index_edits
            .iter()
            .map(|edit| edit.new_text.as_str())
            .collect::<Vec<_>>(),
        vec!["target--deep-details", "target--deep-details"]
    );

    let nested_edits = changes.get(&nested_uri).expect("nested edits");
    assert_eq!(nested_edits.len(), 1);
    assert_eq!(nested_edits[0].new_text, "target--deep-details");
    assert_eq!(changes.len(), 3);

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn renames_page_route_source_file_and_resolved_links() {
    let dir = temp_dir("page-route-rename");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create guide dir");
    std::fs::create_dir_all(content_dir.join("nested")).expect("create nested dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write guide");
    std::fs::write(content_dir.join("nested/_index.md"), "# Nested\n").expect("write nested");
    std::fs::write(
            templates_dir.join("page.html"),
            "<nav><a href=\"/guide/intro#intro\">Intro</a><a href=\"/elsewhere\">Elsewhere</a></nav>{{ page.content | safe }}",
        )
        .expect("write page template");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "\
+++
title = \"Intro\"
+++

# Intro

[local](#intro)
[absolute self](/guide/intro#intro)
[source self](@/guide/intro.md#intro)
",
    )
    .expect("write target");
    std::fs::write(
        content_dir.join("guide/source.md"),
        "\
# Source

[relative route](intro#intro)
[relative source](intro.md#intro)
[wrong](/guide/other)
",
    )
    .expect("write guide source");
    std::fs::write(
        content_dir.join("nested/source.md"),
        "\
# Source

[absolute](/guide/intro#intro)
[absolute source](@/guide/intro.md#intro)
[relative route](../guide/intro#intro)
[relative source](../guide/intro.md#intro)
",
    )
    .expect("write nested source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let target_page = project
        .page_for_source_file("guide/intro.md")
        .expect("target page");
    let plan = page_route_rename_plan(&content_dir, &project, target_page, "/manual/setup")
        .expect("rename plan")
        .expect("page route rename");

    assert_eq!(plan.old_route, "/guide/intro");
    assert_eq!(plan.new_route, "/manual/setup");
    assert_eq!(plan.old_source_file, "guide/intro.md");
    assert_eq!(plan.new_source_file, "manual/setup.md");
    assert_eq!(
        plan.text_edits
            .iter()
            .map(|edit| (
                page_route_text_edit_sort_key(&edit.path),
                edit.new_target.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "source:guide/source.md".to_string(),
                "../manual/setup#intro"
            ),
            (
                "source:guide/source.md".to_string(),
                "../manual/setup.md#intro"
            ),
            ("source:manual/setup.md".to_string(), "/manual/setup#intro"),
            (
                "source:manual/setup.md".to_string(),
                "@/manual/setup.md#intro"
            ),
            ("source:nested/source.md".to_string(), "/manual/setup#intro"),
            (
                "source:nested/source.md".to_string(),
                "@/manual/setup.md#intro"
            ),
            (
                "source:nested/source.md".to_string(),
                "../manual/setup#intro"
            ),
            (
                "source:nested/source.md".to_string(),
                "../manual/setup.md#intro"
            ),
            ("template:page.html".to_string(), "/manual/setup#intro"),
        ]
    );

    let workspace_edit =
        workspace_edit_for_page_route_rename(&content_dir, &plan).expect("workspace edit");
    let operations = match workspace_edit.document_changes.expect("document changes") {
        DocumentChanges::Operations(operations) => operations,
        DocumentChanges::Edits(_) => panic!("expected document change operations"),
    };
    match &operations[0] {
        DocumentChangeOperation::Op(ResourceOp::Rename(rename)) => {
            assert_eq!(
                rename.old_uri,
                Url::from_file_path(content_dir.join("guide/intro.md")).expect("old uri")
            );
            assert_eq!(
                rename.new_uri,
                Url::from_file_path(content_dir.join("manual/setup.md")).expect("new uri")
            );
        }
        _ => panic!("expected file rename operation"),
    }
    assert_eq!(operations.len(), 5);

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn completes_routes_sources_static_assets_and_headings_from_authoring_project() {
    let dir = temp_dir("completions");
    let content_dir = dir.join("content");
    let static_dir = dir.join("static");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write section");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "# Intro\n\n## Details\n",
    )
    .expect("write intro");
    let source = "\
# Source

[absolute](/)
[source](@/)
[relative]()
[heading](/guide/intro#)
[local](#)
![image](/)
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let page = project
        .page_for_source_file("guide/source.md")
        .expect("source page");
    let contexts = markdown_target_contexts(source);

    assert!(completion_new_texts(&project, page, &contexts[0]).contains(&"/guide/intro".into()));
    assert!(
        completion_new_texts(&project, page, &contexts[1]).contains(&"@/guide/intro.md".into())
    );
    assert!(completion_new_texts(&project, page, &contexts[2]).contains(&"intro".into()));
    assert!(
        completion_new_texts(&project, page, &contexts[3])
            .contains(&"/guide/intro#intro--details".into())
    );
    assert!(completion_new_texts(&project, page, &contexts[4]).contains(&"#source".into()));
    assert!(completion_new_texts(&project, page, &contexts[5]).contains(&"/logo.png".into()));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

fn completion_new_texts(
    project: &AuthoringProject,
    page: &AuthoringPage,
    context: &MarkdownTargetContext,
) -> Vec<String> {
    completion_items_for_markdown_target(project, page, context)
        .into_iter()
        .filter_map(|item| match item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => Some(edit.new_text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn hovers_links_and_frontmatter_from_authoring_project() {
    let dir = temp_dir("hovers");
    let content_dir = dir.join("content");
    let static_dir = dir.join("static");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n\n## Details\n",
    )
    .expect("write intro");
    let source = "\
+++
title = \"Source\"
+++

[intro](/guide/intro#intro--details)
[missing](/missing)
![logo](/logo.png)
";
    std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

    let project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load project");
    let template_index = TemplateAuthoringIndex::new(&project);
    let page = project
        .page_for_source_file("guide/source.md")
        .expect("source page");

    let intro_reference = reference_at_position(source, position_for(source, "/guide/intro"))
        .expect("intro reference");
    let intro_hover = link_hover_markdown(&project, page, source, &intro_reference);
    assert!(intro_hover.contains("**Dodeca Intro**"));
    assert!(intro_hover.contains("> # Intro ## Details"));
    assert!(intro_hover.contains("**Heading**: H2 `Details` (`#intro--details`)"));
    assert!(intro_hover.contains("| `/guide/intro` | `guide/intro.md` |"));

    let missing_reference =
        reference_at_position(source, position_for(source, "/missing")).expect("missing reference");
    let missing_hover = link_hover_markdown(&project, page, source, &missing_reference);
    assert!(missing_hover.contains("route '/missing' not found"));

    let logo_reference =
        reference_at_position(source, position_for(source, "/logo.png")).expect("logo reference");
    let logo_hover = link_hover_markdown(&project, page, source, &logo_reference);
    assert!(logo_hover.contains("Dodeca static asset"));
    assert!(logo_hover.contains("static/logo.png"));

    let frontmatter_hover = frontmatter_hover_markdown(&project, &template_index, page, source, 3);
    assert!(frontmatter_hover.contains("**Dodeca page: Source**"));
    assert!(frontmatter_hover.contains("> [intro](/guide/intro#intro--details)"));
    assert!(frontmatter_hover.contains("| route | source | headings | backlinks |"));
    assert!(frontmatter_hover.contains("| `/guide/source` | `guide/source.md` |"));
    assert!(frontmatter_hover.contains("| transforms | `markdown -> page template"));
    assert!(frontmatter_hover.contains("| static assets | `logo.png` |"));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn symbols_list_pages_and_headings_from_authoring_project() {
    let dir = temp_dir("symbols");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
    std::fs::write(
        content_dir.join("guide/intro.md"),
        "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n\n## Details\n",
    )
    .expect("write intro");
    std::fs::write(content_dir.join("guide/source.md"), "# Source\n").expect("write source");

    let project = load_authoring_project(
        &content_dir,
        &[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Source("guide/draft.md".to_string()),
            content: "+++\ntitle = \"Draft\"\n+++\n\n# Draft\n".to_string(),
        }],
    )
    .await
    .expect("load project");
    let intro = project
        .page_for_source_file("guide/intro.md")
        .expect("intro page");
    let intro_content = project
        .source_contents
        .get("guide/intro.md")
        .expect("intro content");

    let document_symbols = document_symbol_for_page(intro, intro_content);
    let children = document_symbols.children.expect("heading children");
    assert_eq!(document_symbols.name, "Intro");
    assert_eq!(
        children
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Intro", "Details"]
    );

    let route_symbols = workspace_symbols_for_project(&content_dir, &project, "/guide/intro");
    assert!(route_symbols.iter().any(|symbol| symbol.name == "Intro"));

    let source_symbols = workspace_symbols_for_project(&content_dir, &project, "draft.md");
    assert!(source_symbols.iter().any(|symbol| symbol.name == "Draft"));

    let heading_symbols = workspace_symbols_for_project(&content_dir, &project, "intro--details");
    assert!(heading_symbols.iter().any(|symbol| {
        symbol.name == "Details"
            && symbol
                .container_name
                .as_deref()
                .is_some_and(|container| container.contains("/guide/intro"))
    }));

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn authoring_project_uses_open_document_overlays() {
    let dir = temp_dir("overlays");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::write(content_dir.join("_index.md"), "[target](/target)\n").expect("write root");
    std::fs::write(content_dir.join("target.md"), "# Target\n").expect("write target");

    let disk_project = load_authoring_project(&content_dir, &[])
        .await
        .expect("load disk project");
    assert!(disk_project.route_exists("/target"));

    let overlay_project = load_authoring_project(
        &content_dir,
        &[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Source("draft.md".to_string()),
            content: "+++\ntitle = \"Draft\"\n+++\n".to_string(),
        }],
    )
    .await
    .expect("load overlay project");

    assert!(overlay_project.route_exists("/target"));
    assert!(overlay_project.route_exists("/draft"));
    assert_eq!(
        overlay_project.source_file_for_route("/draft"),
        Some("draft.md")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn authoring_workspace_updates_open_document_overlays() {
    let dir = temp_dir("workspace-overlays");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
    assert!(
        !workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .route_exists("/draft")
    );

    workspace
        .apply_overlays(&[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Source("draft.md".to_string()),
            content: "+++\ntitle = \"Draft\"\n+++\n".to_string(),
        }])
        .expect("apply overlay");
    assert!(
        workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .route_exists("/draft")
    );

    workspace.apply_overlays(&[]).expect("clear overlay");
    assert!(
        !workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .route_exists("/draft")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn authoring_workspace_updates_template_overlays() {
    let dir = temp_dir("workspace-template-overlays");
    let content_dir = dir.join("content");
    let templates_dir = dir.join("templates");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::create_dir_all(&templates_dir).expect("create templates dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
    assert!(
        !workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .template_paths
            .contains_key("custom.html")
    );

    workspace
        .apply_overlays(&[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Template("custom.html".to_string()),
            content: "{% set alpha = \"/\" %}{{ alpha }}".to_string(),
        }])
        .expect("apply template overlay");
    let alpha_project = workspace.inputs().project().await.expect("alpha project");
    assert!(alpha_project.template_paths.contains_key("custom.html"));
    let alpha_index = alpha_project
        .template_semantics
        .get("custom.html")
        .expect("alpha template semantics");
    assert!(
        alpha_index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "alpha")
    );

    workspace
        .apply_overlays(&[AuthoringDocumentOverlay {
            path: AuthoringInputPath::Template("custom.html".to_string()),
            content: "{% set beta = \"/\" %}{{ beta".to_string(),
        }])
        .expect("update template overlay");
    let beta_project = workspace.inputs().project().await.expect("beta project");
    let beta_index = beta_project
        .template_semantics
        .get("custom.html")
        .expect("beta template semantics");
    assert!(
        beta_index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "beta")
    );
    assert!(
        !beta_index
            .symbols
            .iter()
            .any(|symbol| symbol.name == "alpha")
    );

    workspace.apply_overlays(&[]).expect("clear overlay");
    let cleared_project = workspace.inputs().project().await.expect("cleared project");
    assert!(!cleared_project.template_paths.contains_key("custom.html"));
    assert!(
        !cleared_project
            .template_semantics
            .contains_key("custom.html")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[tokio::test]
async fn authoring_workspace_updates_disk_source_changes() {
    let dir = temp_dir("workspace-disk-source-changes");
    let content_dir = dir.join("content");
    std::fs::create_dir_all(&content_dir).expect("create content dir");
    std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

    let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
    workspace
        .apply_file_change(
            &AuthoringInputPath::Source("draft.md".to_string()),
            Some("+++\ntitle = \"Draft\"\n+++\n"),
        )
        .expect("apply source change");
    assert!(
        workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .route_exists("/draft")
    );

    workspace
        .apply_file_change(&AuthoringInputPath::Source("draft.md".to_string()), None)
        .expect("apply source removal");
    assert!(
        !workspace
            .inputs()
            .project()
            .await
            .expect("project")
            .route_exists("/draft")
    );

    std::fs::remove_dir_all(&dir).expect("remove temp dir");
}
