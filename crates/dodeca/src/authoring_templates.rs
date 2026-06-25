//! Template authoring analysis: the index of blocks/macros/includes/route refs
//! and template-file diagnostics, plus frontmatter document targets. Lifted out
//! of `dodeca-authoring-lsp` so it can back picante tracked queries.
//!
//! These types embed `lsp_types::Range` (vendored to derive `Facet`); a protocol
//! type in the engine is a known layering smell, accepted to avoid migrating the
//! whole subsystem to a neutral range type. The editor crate re-exports
//! everything here, so its call sites are unchanged.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use camino::Utf8PathBuf;
use eyre::{Result, eyre};
use gingembre::ast::{Expr, Ident, Node, StringLit};
use lsp_types::{Position, Range};
use url::Url;

use crate::authoring_graph::{byte_to_line_column, rendered_href_target_route};
use crate::authoring_model::{AuthoringInputPath, AuthoringProject};

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TemplateRouteReference {
    pub target: String,
    pub target_route: String,
    pub source_range: Range,
}

#[derive(Debug, Clone, facet::Facet)]
pub struct TemplateBlockOccurrence {
    pub name: String,
    pub source_range: Range,
}

#[derive(Debug, Clone, facet::Facet)]
pub struct TemplateMacroOccurrence {
    pub name: String,
    pub source_range: Range,
}

#[derive(Debug, Clone, facet::Facet)]
pub struct TemplateMacroCallOccurrence {
    pub target_template_file: String,
    pub macro_name: String,
    pub source_range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TemplateBlockReferenceTarget {
    pub path: Utf8PathBuf,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TemplateDocumentReferenceTarget {
    pub path: Utf8PathBuf,
    pub range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TemplateMacroReferenceQuery {
    pub target_template_file: String,
    pub macro_name: String,
    pub source_range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TemplateMacroReferenceTarget {
    pub path: Utf8PathBuf,
    pub range: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum TemplateDocumentKind {
    Extends,
    Include,
    Import,
}

impl TemplateDocumentKind {
    pub fn label(self) -> &'static str {
        match self {
            TemplateDocumentKind::Extends => "extends",
            TemplateDocumentKind::Include => "include",
            TemplateDocumentKind::Import => "import",
        }
    }
}

#[derive(Debug, Clone, facet::Facet)]
pub struct TemplateDocumentTarget {
    pub kind: TemplateDocumentKind,
    pub path: String,
    pub target_path: Utf8PathBuf,
    pub source_range: Range,
}

impl TemplateDocumentTarget {
    pub fn target_uri(&self) -> Result<Url> {
        Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert template path to URI: {}",
                self.target_path
            )
        })
    }

    pub fn tooltip(&self) -> String {
        format!("Open Dodeca template {} `{}`", self.kind.label(), self.path)
    }

    pub fn hover_markdown(&self) -> String {
        format!(
            "**Dodeca template {}**\n\n`{}`\n\nSource: `{}`",
            self.kind.label(),
            self.path,
            self.target_path
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(u8)]
pub enum FrontmatterDocumentKind {
    Template,
    StaticAsset,
    DataFile,
}

impl FrontmatterDocumentKind {
    pub fn label(self) -> &'static str {
        match self {
            FrontmatterDocumentKind::Template => "template",
            FrontmatterDocumentKind::StaticAsset => "static asset",
            FrontmatterDocumentKind::DataFile => "data file",
        }
    }
}

#[derive(Debug, Clone, facet::Facet)]
pub struct FrontmatterDocumentTarget {
    pub kind: FrontmatterDocumentKind,
    pub path: String,
    pub target_path: Utf8PathBuf,
    pub source_range: Range,
}

impl FrontmatterDocumentTarget {
    pub fn target_uri(&self) -> Result<Url> {
        Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert {} path to URI: {}",
                self.kind.label(),
                self.target_path
            )
        })
    }

    pub fn tooltip(&self) -> String {
        format!("Open Dodeca {} `{}`", self.kind.label(), self.path)
    }

    pub fn hover_markdown(&self) -> String {
        format!(
            "**Dodeca {}**\n\n`{}`\n\nSource: `{}`",
            self.kind.label(),
            self.path,
            self.target_path
        )
    }
}

pub fn byte_range_to_lsp_range(content: &str, byte_start: usize, byte_end: usize) -> Range {
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    Range {
        start: Position {
            line: line.saturating_sub(1),
            character: column.saturating_sub(1),
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: column_end.saturating_sub(1),
        },
    }
}

pub fn template_string_range(content: &str, string: &StringLit) -> Range {
    byte_range_to_lsp_range(
        content,
        string.span.offset(),
        string.span.offset() + string.span.len(),
    )
}

pub fn template_ident_range(content: &str, ident: &Ident) -> Range {
    byte_range_to_lsp_range(
        content,
        ident.span.offset(),
        ident.span.offset() + ident.span.len(),
    )
}

pub fn range_contains_position(range: &Range, position: Position) -> bool {
    position_le(range.start, position) && position_le(position, range.end)
}

pub fn position_le(left: Position, right: Position) -> bool {
    left.line < right.line || (left.line == right.line && left.character <= right.character)
}

pub fn position_cmp(left: Position, right: Position) -> Ordering {
    left.line
        .cmp(&right.line)
        .then_with(|| left.character.cmp(&right.character))
}

pub fn template_route_references(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Vec<TemplateRouteReference> {
    let mut references = Vec::new();
    let mut seen = HashSet::new();

    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(origin) = &href.origin else {
                continue;
            };
            let AuthoringInputPath::Template(origin_template_file) = &origin.path else {
                continue;
            };
            if origin_template_file != template_file || origin.byte_end > content.len() {
                continue;
            }
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if !seen.insert((
                origin.byte_start,
                origin.byte_end,
                href.href.clone(),
                target_route.clone(),
            )) {
                continue;
            }
            references.push(TemplateRouteReference {
                target: href.href.clone(),
                target_route,
                source_range: byte_range_to_lsp_range(content, origin.byte_start, origin.byte_end),
            });
        }
    }

    references
}
pub fn template_path_dependencies(nodes: &[Node]) -> Vec<String> {
    let mut dependencies = Vec::new();
    collect_template_path_dependencies(nodes, &mut dependencies);
    dependencies
}
pub fn collect_template_path_dependencies(nodes: &[Node], dependencies: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Extends(node) => dependencies.push(node.path.value.clone()),
            Node::Include(node) => dependencies.push(node.path.value.clone()),
            Node::Import(node) => dependencies.push(node.path.value.clone()),
            Node::If(node) => {
                collect_template_path_dependencies(&node.then_body, dependencies);
                for branch in &node.elif_branches {
                    collect_template_path_dependencies(&branch.body, dependencies);
                }
                if let Some(body) = &node.else_body {
                    collect_template_path_dependencies(body, dependencies);
                }
            }
            Node::For(node) => {
                collect_template_path_dependencies(&node.body, dependencies);
                if let Some(body) = &node.else_body {
                    collect_template_path_dependencies(body, dependencies);
                }
            }
            Node::Block(node) => collect_template_path_dependencies(&node.body, dependencies),
            Node::Macro(node) => collect_template_path_dependencies(&node.body, dependencies),
            Node::Text(_)
            | Node::Print(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}
pub fn template_document_targets_for_nodes(
    project: &AuthoringProject,
    content: &str,
    nodes: &[Node],
) -> Vec<TemplateDocumentTarget> {
    let mut targets = Vec::new();
    collect_template_document_targets(project, content, nodes, &mut targets);
    targets
}
pub fn collect_template_document_targets(
    project: &AuthoringProject,
    content: &str,
    nodes: &[Node],
    targets: &mut Vec<TemplateDocumentTarget>,
) {
    for node in nodes {
        match node {
            Node::Extends(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Extends,
                &node.path,
                targets,
            ),
            Node::Include(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Include,
                &node.path,
                targets,
            ),
            Node::Import(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Import,
                &node.path,
                targets,
            ),
            Node::If(node) => {
                collect_template_document_targets(project, content, &node.then_body, targets);
                for branch in &node.elif_branches {
                    collect_template_document_targets(project, content, &branch.body, targets);
                }
                if let Some(body) = &node.else_body {
                    collect_template_document_targets(project, content, body, targets);
                }
            }
            Node::For(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
                if let Some(body) = &node.else_body {
                    collect_template_document_targets(project, content, body, targets);
                }
            }
            Node::Block(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
            }
            Node::Macro(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}
pub fn push_template_document_target(
    project: &AuthoringProject,
    content: &str,
    kind: TemplateDocumentKind,
    path: &StringLit,
    targets: &mut Vec<TemplateDocumentTarget>,
) {
    let Some(target_path) = project.template_paths.get(&path.value) else {
        return;
    };
    targets.push(TemplateDocumentTarget {
        kind,
        path: path.value.clone(),
        target_path: target_path.clone(),
        source_range: template_string_range(content, path),
    });
}
pub fn template_block_occurrences(content: &str, nodes: &[Node]) -> Vec<TemplateBlockOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_block_occurrences(content, nodes, &mut occurrences);
    occurrences
}
pub fn collect_template_block_occurrences(
    content: &str,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateBlockOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Block(node) => {
                occurrences.push(TemplateBlockOccurrence {
                    name: node.name.name.clone(),
                    source_range: template_ident_range(content, &node.name),
                });
                collect_template_block_occurrences(content, &node.body, occurrences);
            }
            Node::If(node) => {
                collect_template_block_occurrences(content, &node.then_body, occurrences);
                for branch in &node.elif_branches {
                    collect_template_block_occurrences(content, &branch.body, occurrences);
                }
                if let Some(body) = &node.else_body {
                    collect_template_block_occurrences(content, body, occurrences);
                }
            }
            Node::For(node) => {
                collect_template_block_occurrences(content, &node.body, occurrences);
                if let Some(body) = &node.else_body {
                    collect_template_block_occurrences(content, body, occurrences);
                }
            }
            Node::Macro(node) => {
                collect_template_block_occurrences(content, &node.body, occurrences)
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}
pub fn template_macro_occurrences(content: &str, nodes: &[Node]) -> Vec<TemplateMacroOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_macro_occurrences(content, nodes, &mut occurrences);
    occurrences
}
pub fn collect_template_macro_occurrences(
    content: &str,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateMacroOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Macro(node) => {
                occurrences.push(TemplateMacroOccurrence {
                    name: node.name.name.clone(),
                    source_range: template_ident_range(content, &node.name),
                });
                collect_template_macro_occurrences(content, &node.body, occurrences);
            }
            Node::If(node) => {
                collect_template_macro_occurrences(content, &node.then_body, occurrences);
                for branch in &node.elif_branches {
                    collect_template_macro_occurrences(content, &branch.body, occurrences);
                }
                if let Some(body) = &node.else_body {
                    collect_template_macro_occurrences(content, body, occurrences);
                }
            }
            Node::For(node) => {
                collect_template_macro_occurrences(content, &node.body, occurrences);
                if let Some(body) = &node.else_body {
                    collect_template_macro_occurrences(content, body, occurrences);
                }
            }
            Node::Block(node) => {
                collect_template_macro_occurrences(content, &node.body, occurrences);
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}
pub fn template_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    nodes: &[Node],
) -> Vec<TemplateMacroCallOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_macro_call_occurrences(
        template_file,
        content,
        imports,
        nodes,
        &mut occurrences,
    );
    occurrences
}
pub fn collect_template_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Print(node) => collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.expr,
                occurrences,
            ),
            Node::If(node) => {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.condition,
                    occurrences,
                );
                collect_template_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.then_body,
                    occurrences,
                );
                for branch in &node.elif_branches {
                    collect_expr_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        &branch.condition,
                        occurrences,
                    );
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        &branch.body,
                        occurrences,
                    );
                }
                if let Some(body) = &node.else_body {
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        body,
                        occurrences,
                    );
                }
            }
            Node::For(node) => {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.iter,
                    occurrences,
                );
                collect_template_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.body,
                    occurrences,
                );
                if let Some(body) = &node.else_body {
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        body,
                        occurrences,
                    );
                }
            }
            Node::Block(node) => collect_template_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.body,
                occurrences,
            ),
            Node::Set(node) => collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.value,
                occurrences,
            ),
            Node::Macro(node) => collect_template_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.body,
                occurrences,
            ),
            Node::CallBlock(node) => {
                for (_, expr) in &node.kwargs {
                    collect_expr_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        expr,
                        occurrences,
                    );
                }
            }
            Node::Text(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_) => {}
        }
    }
}
pub fn collect_expr_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    expr: &Expr,
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    match expr {
        Expr::Optional(inner) => collect_expr_macro_call_occurrences(
            template_file,
            content,
            imports,
            &inner.expr,
            occurrences,
        ),
        Expr::MacroCall(expr) => {
            let target_file = if expr.namespace.name == "self" {
                Some(template_file)
            } else {
                imports.get(&expr.namespace.name).map(|path| path.as_str())
            };
            if let Some(target_file) = target_file {
                occurrences.push(TemplateMacroCallOccurrence {
                    target_template_file: target_file.to_string(),
                    macro_name: expr.macro_name.name.clone(),
                    source_range: template_ident_range(content, &expr.macro_name),
                });
            }
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Field(expr) => collect_expr_macro_call_occurrences(
            template_file,
            content,
            imports,
            &expr.base,
            occurrences,
        ),
        Expr::Index(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.base,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.index,
                occurrences,
            );
        }
        Expr::Filter(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.expr,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Binary(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.left,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.right,
                occurrences,
            );
        }
        Expr::Unary(expr) => collect_expr_macro_call_occurrences(
            template_file,
            content,
            imports,
            &expr.expr,
            occurrences,
        ),
        Expr::Call(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.func,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Ternary(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.value,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.condition,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.otherwise,
                occurrences,
            );
        }
        Expr::Test(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.expr,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
        }
        Expr::Literal(literal) => {
            collect_literal_macro_call_occurrences(
                template_file,
                content,
                imports,
                literal,
                occurrences,
            );
        }
        Expr::Var(_) => {}
    }
}
pub fn template_import_aliases(
    project: &AuthoringProject,
    nodes: &[Node],
) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    collect_template_import_aliases(project, nodes, &mut imports);
    imports
}
pub fn collect_template_import_aliases(
    project: &AuthoringProject,
    nodes: &[Node],
    imports: &mut HashMap<String, String>,
) {
    for node in nodes {
        match node {
            Node::Import(node) => {
                if project.template_paths.contains_key(&node.path.value) {
                    imports.insert(node.alias.name.clone(), node.path.value.clone());
                }
            }
            Node::If(node) => {
                collect_template_import_aliases(project, &node.then_body, imports);
                for branch in &node.elif_branches {
                    collect_template_import_aliases(project, &branch.body, imports);
                }
                if let Some(body) = &node.else_body {
                    collect_template_import_aliases(project, body, imports);
                }
            }
            Node::For(node) => {
                collect_template_import_aliases(project, &node.body, imports);
                if let Some(body) = &node.else_body {
                    collect_template_import_aliases(project, body, imports);
                }
            }
            Node::Block(node) => collect_template_import_aliases(project, &node.body, imports),
            Node::Macro(node) => collect_template_import_aliases(project, &node.body, imports),
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}
pub fn template_extends_path_from_nodes(nodes: &[Node]) -> Option<String> {
    for node in nodes {
        match node {
            Node::Extends(node) => return Some(node.path.value.clone()),
            Node::Text(node) if node.text.trim().is_empty() => {}
            _ => return None,
        }
    }
    None
}

pub fn collect_literal_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    literal: &gingembre::ast::Literal,
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    match literal {
        gingembre::ast::Literal::List(list) => {
            for expr in &list.elements {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        gingembre::ast::Literal::Dict(dict) => {
            for (key, value) in &dict.entries {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    key,
                    occurrences,
                );
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    value,
                    occurrences,
                );
            }
        }
        gingembre::ast::Literal::String(_)
        | gingembre::ast::Literal::Int(_)
        | gingembre::ast::Literal::Float(_)
        | gingembre::ast::Literal::Bool(_)
        | gingembre::ast::Literal::None(_) => {}
    }
}
