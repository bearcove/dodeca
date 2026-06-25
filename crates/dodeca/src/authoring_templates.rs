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

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::{Facet, NumericType, PrimitiveType, Type, UserType};
use gingembre::ast::{Expr, Ident, Node, StringLit};
use gingembre::semantic::{TemplateSemanticIndex, TemplateSemanticTokenKind};
use gingembre::{BuiltinItemInfo, builtin_filter, builtin_test};
use lsp_types::{Location, Position, Range, SemanticToken, SemanticTokens};
use url::Url;

use crate::authoring_graph::{byte_to_line_column, rendered_href_target_route};
use crate::authoring_model::{
    AuthoringDiagnostic, AuthoringDiagnosticKind, AuthoringInputPath, AuthoringProject,
};
use crate::queries::Frontmatter;

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

pub fn diagnostics_for_template_nodes(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    nodes: &[Node],
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    let imports = template_import_aliases(project, nodes);
    let parent_file = template_extends_path_from_nodes(nodes)
        .filter(|path| project.template_paths.contains_key(path));
    collect_template_diagnostics(
        project,
        template_file,
        content,
        nodes,
        parent_file.as_deref(),
        &imports,
        &mut diagnostics,
    );
    diagnostics
}
pub fn collect_template_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    nodes: &[Node],
    parent_file: Option<&str>,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    for node in nodes {
        match node {
            Node::Extends(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Extends,
                &node.path,
                diagnostics,
            ),
            Node::Include(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Include,
                &node.path,
                diagnostics,
            ),
            Node::Import(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Import,
                &node.path,
                diagnostics,
            ),
            Node::Block(node) => {
                if let Some(parent_file) = parent_file
                    && template_block_definition(
                        project,
                        parent_file,
                        template_file,
                        content,
                        &node.name.name,
                    )
                    .is_none()
                {
                    diagnostics.push(template_diagnostic_for_ident(
                        template_file,
                        content,
                        AuthoringDiagnosticKind::MissingBlock,
                        &node.name,
                        format!("parent block '{}' not found", node.name.name),
                    ));
                }
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
            }
            Node::Macro(node) => {
                for param in &node.params {
                    if let Some(default) = &param.default {
                        collect_template_expr_diagnostics(
                            project,
                            template_file,
                            content,
                            default,
                            imports,
                            diagnostics,
                        );
                    }
                }
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
            }
            Node::Print(node) => collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &node.expr,
                imports,
                diagnostics,
            ),
            Node::If(node) => {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.condition,
                    imports,
                    diagnostics,
                );
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.then_body,
                    parent_file,
                    imports,
                    diagnostics,
                );
                for branch in &node.elif_branches {
                    collect_template_expr_diagnostics(
                        project,
                        template_file,
                        content,
                        &branch.condition,
                        imports,
                        diagnostics,
                    );
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        &branch.body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
                if let Some(body) = &node.else_body {
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::For(node) => {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.iter,
                    imports,
                    diagnostics,
                );
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
                if let Some(body) = &node.else_body {
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::Set(node) => collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &node.value,
                imports,
                diagnostics,
            ),
            Node::CallBlock(node) => {
                for (_, expr) in &node.kwargs {
                    collect_template_expr_diagnostics(
                        project,
                        template_file,
                        content,
                        expr,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::Text(_) | Node::Comment(_) | Node::Continue(_) | Node::Break(_) => {}
        }
    }
}
pub fn push_missing_template_diagnostic(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    kind: TemplateDocumentKind,
    path: &StringLit,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    if project.template_paths.contains_key(&path.value) {
        return;
    }
    diagnostics.push(template_diagnostic_for_span(
        template_file,
        content,
        AuthoringDiagnosticKind::MissingTemplate,
        &path.value,
        format!("template {} '{}' not found", kind.label(), path.value),
        path.span.offset(),
        path.span.offset() + path.span.len(),
    ));
}
pub fn collect_template_expr_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    expr: &Expr,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    match expr {
        Expr::Optional(inner) => collect_template_expr_diagnostics(
            project,
            template_file,
            content,
            &inner.expr,
            imports,
            diagnostics,
        ),
        Expr::Literal(literal) => collect_template_literal_diagnostics(
            project,
            template_file,
            content,
            literal,
            imports,
            diagnostics,
        ),
        Expr::Var(_) => {}
        Expr::Field(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                diagnostics,
            );
        }
        Expr::Index(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.index,
                imports,
                diagnostics,
            );
        }
        Expr::Filter(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
            if builtin_filter(&expr.filter.name).is_none() {
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownFilter,
                    &expr.filter,
                    format!("filter '{}' not found", expr.filter.name),
                ));
            }
        }
        Expr::Binary(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.left,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.right,
                imports,
                diagnostics,
            );
        }
        Expr::Unary(expr) => collect_template_expr_diagnostics(
            project,
            template_file,
            content,
            &expr.expr,
            imports,
            diagnostics,
        ),
        Expr::Call(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.func,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
        }
        Expr::Ternary(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.value,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.condition,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.otherwise,
                imports,
                diagnostics,
            );
        }
        Expr::Test(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            if builtin_test(&expr.test_name.name).is_none() {
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownTest,
                    &expr.test_name,
                    format!("test '{}' not found", expr.test_name.name),
                ));
            }
        }
        Expr::MacroCall(expr) => {
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
            if template_macro_definition_target(
                project,
                template_file,
                content,
                imports,
                &expr.namespace,
                &expr.macro_name,
            )
            .is_none()
            {
                let target = format!("{}::{}", expr.namespace.name, expr.macro_name.name);
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownMacro,
                    &expr.macro_name,
                    format!("macro '{target}' not found"),
                ));
            }
        }
    }
}
pub fn collect_template_literal_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    literal: &gingembre::ast::Literal,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    match literal {
        gingembre::ast::Literal::List(list) => {
            for expr in &list.elements {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
        }
        gingembre::ast::Literal::Dict(dict) => {
            for (key, value) in &dict.entries {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    key,
                    imports,
                    diagnostics,
                );
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    value,
                    imports,
                    diagnostics,
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
pub fn template_diagnostic_for_ident(
    template_file: &str,
    content: &str,
    kind: AuthoringDiagnosticKind,
    ident: &Ident,
    message: String,
) -> AuthoringDiagnostic {
    template_diagnostic_for_span(
        template_file,
        content,
        kind,
        &ident.name,
        message,
        ident.span.offset(),
        ident.span.offset() + ident.span.len(),
    )
}
pub fn template_diagnostic_for_span(
    template_file: &str,
    content: &str,
    kind: AuthoringDiagnosticKind,
    target: &str,
    message: String,
    byte_start: usize,
    byte_end: usize,
) -> AuthoringDiagnostic {
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    AuthoringDiagnostic {
        source_file: template_file.to_string(),
        route: String::new(),
        kind,
        target: target.to_string(),
        resolved_route: None,
        message,
        line,
        column,
        line_end,
        column_end,
        byte_start,
        byte_end,
    }
}
pub fn template_block_definition(
    project: &AuthoringProject,
    template_file: &str,
    current_file: &str,
    current_content: &str,
    name: &str,
) -> Option<(String, String, Ident)> {
    let content = template_content(project, template_file, current_file, current_content)?;
    let template = gingembre::parse_template(template_file, content.as_str()).ok()?;
    if let Some(ident) = top_level_block_ident(&template.body, name) {
        return Some((template_file.to_string(), content, ident));
    }

    let mut seen = HashSet::new();
    if let Some(parent_file) = template_extends_path(template_file, &content, &mut seen) {
        return template_block_definition(
            project,
            &parent_file,
            current_file,
            current_content,
            name,
        );
    }

    None
}

pub fn template_extends_path(
    template_file: &str,
    content: &str,
    seen: &mut HashSet<String>,
) -> Option<String> {
    if !seen.insert(template_file.to_string()) {
        return None;
    }
    let template = gingembre::parse_template(template_file, content).ok()?;
    template_extends_path_from_nodes(&template.body)
}
pub fn template_macro_definition_target(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    namespace: &Ident,
    macro_name: &Ident,
) -> Option<TemplateDefinitionTarget> {
    let target_file = if namespace.name == "self" {
        template_file
    } else {
        imports.get(&namespace.name)?.as_str()
    };
    let target_content = template_content(project, target_file, template_file, content)?;
    let template = gingembre::parse_template(target_file, target_content.as_str()).ok()?;
    let target_ident = top_level_macro_ident(&template.body, &macro_name.name)?;
    Some(TemplateDefinitionTarget {
        kind: TemplateDefinitionKind::Macro,
        name: format!("{}::{}", namespace.name, macro_name.name),
        source_range: template_ident_range(content, macro_name),
        target_path: project.template_paths.get(target_file).cloned()?,
        target_range: template_ident_range(&target_content, &target_ident),
    })
}
pub fn template_content(
    project: &AuthoringProject,
    template_file: &str,
    current_file: &str,
    current_content: &str,
) -> Option<String> {
    if template_file == current_file {
        Some(current_content.to_string())
    } else {
        project.template_contents.get(template_file).cloned()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TemplateItemInfo {
    pub detail: &'static str,
    pub documentation: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateDefinitionKind {
    Block,
    Macro,
    Filter,
    Test,
}

impl TemplateDefinitionKind {
    pub fn label(self) -> &'static str {
        match self {
            TemplateDefinitionKind::Block => "block",
            TemplateDefinitionKind::Macro => "macro",
            TemplateDefinitionKind::Filter => "filter",
            TemplateDefinitionKind::Test => "test",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemplateDefinitionTarget {
    pub kind: TemplateDefinitionKind,
    pub name: String,
    pub source_range: Range,
    pub target_path: Utf8PathBuf,
    pub target_range: Range,
}
impl TemplateDefinitionTarget {
    pub fn location(&self) -> Result<Location> {
        Ok(Location {
            uri: Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
                eyre!(
                    "could not convert template definition path to URI: {}",
                    self.target_path
                )
            })?,
            range: self.target_range,
        })
    }

    pub fn hover_markdown(&self) -> String {
        let info = match self.kind {
            TemplateDefinitionKind::Filter => builtin_filter(&self.name).map(template_builtin_info),
            TemplateDefinitionKind::Test => builtin_test(&self.name).map(template_builtin_info),
            TemplateDefinitionKind::Block | TemplateDefinitionKind::Macro => None,
        };
        if let Some(info) = info {
            return format!(
                "**{}**\n\n`{}`\n\n{}\n\nDefinition: `{}`",
                info.detail, self.name, info.documentation, self.target_path
            );
        }
        format!(
            "**Dodeca template {}**\n\n`{}`\n\nDefinition: `{}`",
            self.kind.label(),
            self.name,
            self.target_path
        )
    }
}

pub fn template_builtin_info(info: &BuiltinItemInfo) -> TemplateItemInfo {
    TemplateItemInfo {
        detail: info.detail,
        documentation: info.documentation,
    }
}

pub fn top_level_block_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Block(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

pub fn top_level_macro_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Macro(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

#[derive(Debug, Clone)]
pub struct FrontmatterEntry {
    pub key: String,
    pub key_start: usize,
    pub key_end: usize,
    pub value: String,
    pub value_start: usize,
    pub value_end: usize,
    pub table: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FrontmatterFieldSpec {
    pub name: &'static str,
    pub kind: FrontmatterFieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterFieldKind {
    String,
    Integer,
    Table,
}

#[derive(Debug, Clone, Copy)]
pub struct FrontmatterContentByteRange {
    pub start: usize,
    pub end: usize,
}

pub fn frontmatter_lsp_range(content: &str) -> Option<Range> {
    content.strip_prefix("+++\n")?;
    let end = content[4..].find("\n+++")? + 4 + "\n+++".len();
    let (line_end, column_end) = byte_to_line_column(content, end);

    Some(Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: column_end.saturating_sub(1),
        },
    })
}
pub fn frontmatter_field_specs() -> Vec<FrontmatterFieldSpec> {
    let fields = match <Frontmatter as Facet>::SHAPE.ty {
        Type::User(UserType::Struct(struct_type)) => struct_type.fields,
        _ => return Vec::new(),
    };

    fields
        .iter()
        .filter_map(|field| {
            let name = field.rename.unwrap_or(field.name);
            frontmatter_field_kind(name, field.shape.get())
                .map(|kind| FrontmatterFieldSpec { name, kind })
        })
        .collect()
}
pub fn frontmatter_field_kind(
    name: &'static str,
    shape: &'static facet::Shape,
) -> Option<FrontmatterFieldKind> {
    if name == "extra" {
        return Some(FrontmatterFieldKind::Table);
    }

    let shape = if shape.type_identifier == "Option" {
        shape.inner.unwrap_or(shape)
    } else {
        shape
    };

    if shape.type_identifier == "String" {
        return Some(FrontmatterFieldKind::String);
    }

    match shape.ty {
        Type::Primitive(PrimitiveType::Textual(_)) => Some(FrontmatterFieldKind::String),
        Type::Primitive(PrimitiveType::Numeric(NumericType::Integer { .. })) => {
            Some(FrontmatterFieldKind::Integer)
        }
        _ => None,
    }
}
pub fn frontmatter_entries(content: &str) -> Vec<FrontmatterEntry> {
    let Some(block) = frontmatter_content_byte_range(content) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut table = None;
    let mut line_start = block.start;

    while line_start < block.end {
        let line_end = content[line_start..block.end]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(block.end);
        let line = &content[line_start..line_end];
        let trimmed = line.trim();

        if let Some(table_name) = frontmatter_table_name(trimmed) {
            table = Some(table_name);
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && let Some(entry) =
                frontmatter_entry_for_line(content, line_start, line, table.clone())
        {
            entries.push(entry);
        }

        line_start = line_end.saturating_add(1);
    }

    entries
}
pub fn frontmatter_entry_for_line(
    content: &str,
    line_start: usize,
    line: &str,
    table: Option<String>,
) -> Option<FrontmatterEntry> {
    let key_start_in_line = line.find(|c: char| !c.is_whitespace())?;
    let key_tail = &line[key_start_in_line..];
    let key_len = key_tail
        .find(|c: char| !is_frontmatter_key_char(c))
        .unwrap_or(key_tail.len());
    if key_len == 0 {
        return None;
    }

    let after_key = key_start_in_line + key_len;
    let equals_offset = line[after_key..].find('=')? + after_key;
    if !line[after_key..equals_offset]
        .chars()
        .all(char::is_whitespace)
    {
        return None;
    }

    let key_start = line_start + key_start_in_line;
    let key_end = key_start + key_len;
    let value_start_in_line =
        equals_offset + 1 + leading_whitespace_len(&line[equals_offset + 1..]);
    let value_end_in_line = line_comment_start(&line[value_start_in_line..])
        .map(|offset| value_start_in_line + offset)
        .unwrap_or(line.len());
    let value_end_in_line = value_end_in_line - trailing_whitespace_len(&line[..value_end_in_line]);
    let value_start = line_start + value_start_in_line;
    let value_end = line_start + value_end_in_line.max(value_start_in_line);

    Some(FrontmatterEntry {
        key: content[key_start..key_end].to_string(),
        key_start,
        key_end,
        value: content[value_start..value_end].to_string(),
        value_start,
        value_end,
        table,
    })
}
pub fn frontmatter_document_targets(
    project: &AuthoringProject,
    content: &str,
) -> Result<Vec<FrontmatterDocumentTarget>> {
    let targets = frontmatter_entries(content)
        .into_iter()
        .filter_map(|entry| {
            let kind = frontmatter_document_kind_for_entry(&entry)?;
            let (path, source_range) = frontmatter_string_value(content, &entry)?;
            let target_path = match kind {
                FrontmatterDocumentKind::Template => project.template_paths.get(&path)?,
                FrontmatterDocumentKind::StaticAsset => {
                    frontmatter_static_target_path(project, &path)?
                }
                FrontmatterDocumentKind::DataFile => frontmatter_data_target_path(project, &path)?,
            };
            Some(FrontmatterDocumentTarget {
                kind,
                path,
                target_path: target_path.clone(),
                source_range,
            })
        })
        .collect();

    Ok(targets)
}
pub fn frontmatter_document_kind_for_entry(
    entry: &FrontmatterEntry,
) -> Option<FrontmatterDocumentKind> {
    if entry.table.is_some() {
        return None;
    }

    match entry.key.as_str() {
        "template" => Some(FrontmatterDocumentKind::Template),
        "asset" => Some(FrontmatterDocumentKind::StaticAsset),
        "data" => Some(FrontmatterDocumentKind::DataFile),
        _ => None,
    }
}
pub fn frontmatter_string_value(
    content: &str,
    entry: &FrontmatterEntry,
) -> Option<(String, Range)> {
    let value = entry.value.trim();
    let leading = entry.value.find(value)?;
    let start = entry.value_start + leading;
    let quote = value.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    if value.as_bytes().last().copied()? != quote || value.len() < 2 {
        return None;
    }

    let inner_start = start + 1;
    let inner_end = start + value.len() - 1;
    Some((
        content[inner_start..inner_end].to_string(),
        byte_range_to_lsp_range(content, inner_start, inner_end),
    ))
}
pub fn frontmatter_value_matches_kind(value: &str, kind: FrontmatterFieldKind) -> bool {
    let value = value.trim();
    match kind {
        FrontmatterFieldKind::String => value.starts_with('"') || value.starts_with('\''),
        FrontmatterFieldKind::Integer => frontmatter_value_is_integer(value),
        FrontmatterFieldKind::Table => true,
    }
}
pub fn frontmatter_value_is_integer(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let mut previous_underscore = false;
    let mut saw_digit = false;

    for ch in value.chars() {
        match ch {
            '_' if saw_digit && !previous_underscore => previous_underscore = true,
            '0'..='9' => {
                saw_digit = true;
                previous_underscore = false;
            }
            _ => return false,
        }
    }

    saw_digit && !previous_underscore
}
pub fn frontmatter_content_byte_range(content: &str) -> Option<FrontmatterContentByteRange> {
    content.strip_prefix("+++\n")?;
    let closing_start = content[4..].find("\n+++")? + 4;
    Some(FrontmatterContentByteRange {
        start: 4,
        end: closing_start,
    })
}
pub fn frontmatter_table_name(trimmed_line: &str) -> Option<String> {
    let inner = trimmed_line.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}
pub fn frontmatter_table_at_offset(content: &str, offset: usize) -> Option<String> {
    let block = frontmatter_content_byte_range(content)?;
    let mut table = None;
    let mut line_start = block.start;
    let limit = offset.min(block.end);

    while line_start < limit {
        let line_end = content[line_start..limit]
            .find('\n')
            .map(|line_end| line_start + line_end)
            .unwrap_or(limit);
        if let Some(table_name) = frontmatter_table_name(content[line_start..line_end].trim()) {
            table = Some(table_name);
        }
        line_start = line_end.saturating_add(1);
    }

    table
}
pub fn frontmatter_has_extra_table(content: &str) -> bool {
    let Some(block) = frontmatter_content_byte_range(content) else {
        return false;
    };
    content[block.start..block.end]
        .lines()
        .filter_map(|line| frontmatter_table_name(line.trim()))
        .any(|table| table == "extra")
}

pub fn frontmatter_static_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .static_paths
        .get(trimmed)
        .or_else(|| project.static_paths.get(path))
}
pub fn frontmatter_data_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .data_paths
        .get(trimmed)
        .or_else(|| project.data_paths.get(path))
}

pub fn is_frontmatter_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}
pub fn leading_whitespace_len(input: &str) -> usize {
    input
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(input.len())
}
pub fn trailing_whitespace_len(input: &str) -> usize {
    input.len() - input.trim_end_matches(char::is_whitespace).len()
}
pub fn line_comment_start(input: &str) -> Option<usize> {
    let mut in_string = false;
    let mut quote = '\0';
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' && quote == '"' {
                escaped = true;
            } else if ch == quote {
                in_string = false;
            }
        } else if ch == '"' || ch == '\'' {
            in_string = true;
            quote = ch;
        } else if ch == '#' {
            return Some(idx);
        }
    }

    None
}

impl FrontmatterFieldKind {
    pub fn description(self) -> &'static str {
        match self {
            FrontmatterFieldKind::String => "a string",
            FrontmatterFieldKind::Integer => "an integer",
            FrontmatterFieldKind::Table => "a table",
        }
    }
}

#[derive(Debug, Clone, facet::Facet)]
pub struct TemplateAuthoringIndex {
    pub templates: HashMap<String, IndexedTemplate>,
    pub children_by_parent: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, facet::Facet)]
pub struct IndexedTemplate {
    pub path: Utf8PathBuf,
    pub content: String,
    pub semantic: Option<TemplateSemanticIndex>,
    pub extends: Option<String>,
    pub dependencies: Vec<String>,
    pub diagnostics: Vec<AuthoringDiagnostic>,
    pub document_targets: Vec<TemplateDocumentTarget>,
    pub route_references: Vec<TemplateRouteReference>,
    pub blocks: Vec<TemplateBlockOccurrence>,
    pub macros: Vec<TemplateMacroOccurrence>,
    pub macro_calls: Vec<TemplateMacroCallOccurrence>,
}

impl TemplateAuthoringIndex {
    pub fn new(project: &AuthoringProject) -> Self {
        let mut templates = HashMap::new();

        for (template_file, template_path) in &project.template_paths {
            let Some(content) = project.template_contents.get(template_file) else {
                continue;
            };
            let Ok(template) = gingembre::parse_template(template_file, content) else {
                continue;
            };
            let imports = template_import_aliases(project, &template.body);
            templates.insert(
                template_file.clone(),
                IndexedTemplate {
                    path: template_path.clone(),
                    content: content.clone(),
                    semantic: project.template_semantics.get(template_file).cloned(),
                    extends: template_extends_path_from_nodes(&template.body),
                    dependencies: template_path_dependencies(&template.body),
                    diagnostics: diagnostics_for_template_nodes(
                        project,
                        template_file,
                        content,
                        &template.body,
                    ),
                    document_targets: template_document_targets_for_nodes(
                        project,
                        content,
                        &template.body,
                    ),
                    route_references: template_route_references(project, template_file, content),
                    blocks: template_block_occurrences(content, &template.body),
                    macros: template_macro_occurrences(content, &template.body),
                    macro_calls: template_macro_call_occurrences(
                        template_file,
                        content,
                        &imports,
                        &template.body,
                    ),
                },
            );
        }

        let mut children_by_parent = HashMap::<String, Vec<String>>::new();
        for (template_file, template) in &templates {
            let Some(parent_file) = &template.extends else {
                continue;
            };
            if templates.contains_key(parent_file) {
                children_by_parent
                    .entry(parent_file.clone())
                    .or_default()
                    .push(template_file.clone());
            }
        }
        for children in children_by_parent.values_mut() {
            children.sort();
        }

        Self {
            templates,
            children_by_parent,
        }
    }

    pub fn block_occurrence_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateBlockOccurrence> {
        self.templates
            .get(template_file)?
            .blocks
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
            .cloned()
    }

    pub fn document_targets(&self, template_file: &str) -> &[TemplateDocumentTarget] {
        self.templates
            .get(template_file)
            .map(|template| template.document_targets.as_slice())
            .unwrap_or_default()
    }

    pub fn document_target_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateDocumentTarget> {
        self.document_targets(template_file)
            .iter()
            .find(|target| range_contains_position(&target.source_range, position))
            .cloned()
    }

    pub fn route_references(&self, template_file: &str) -> &[TemplateRouteReference] {
        self.templates
            .get(template_file)
            .map(|template| template.route_references.as_slice())
            .unwrap_or_default()
    }

    pub fn route_reference_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateRouteReference> {
        self.route_references(template_file)
            .iter()
            .find(|reference| range_contains_position(&reference.source_range, position))
            .cloned()
    }

    pub fn diagnostics(&self, template_file: &str) -> &[AuthoringDiagnostic] {
        self.templates
            .get(template_file)
            .map(|template| template.diagnostics.as_slice())
            .unwrap_or_default()
    }

    pub fn all_diagnostics(&self) -> Vec<AuthoringDiagnostic> {
        let mut diagnostics = self
            .templates
            .values()
            .flat_map(|template| template.diagnostics.iter().cloned())
            .collect::<Vec<_>>();
        diagnostics.sort_by(|a, b| {
            a.source_file
                .cmp(&b.source_file)
                .then_with(|| a.byte_start.cmp(&b.byte_start))
                .then_with(|| a.target.cmp(&b.target))
        });
        diagnostics
    }

    pub fn document_reference_targets(
        &self,
        target_path: &Utf8Path,
    ) -> Vec<TemplateDocumentReferenceTarget> {
        let mut targets = Vec::new();
        for template in self.templates.values() {
            targets.extend(
                template
                    .document_targets
                    .iter()
                    .filter(|target| target.target_path == target_path)
                    .map(|target| TemplateDocumentReferenceTarget {
                        path: template.path.clone(),
                        range: target.source_range,
                    }),
            );
        }
        targets.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| position_cmp(a.range.start, b.range.start))
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        targets.dedup();
        targets
    }

    pub fn block_definition_target(
        &self,
        template_file: &str,
        occurrence: &TemplateBlockOccurrence,
    ) -> Option<TemplateDefinitionTarget> {
        let mut seen = HashSet::new();
        let mut cursor = template_file.to_string();
        while seen.insert(cursor.clone()) {
            let parent_file = self.templates.get(&cursor)?.extends.clone()?;
            let parent = self.templates.get(&parent_file)?;
            if let Some(target) = parent
                .blocks
                .iter()
                .find(|block| block.name == occurrence.name)
            {
                return Some(TemplateDefinitionTarget {
                    kind: TemplateDefinitionKind::Block,
                    name: occurrence.name.clone(),
                    source_range: occurrence.source_range,
                    target_path: parent.path.clone(),
                    target_range: target.source_range,
                });
            }
            cursor = parent_file;
        }
        None
    }

    pub fn block_reference_targets(
        &self,
        template_file: &str,
        block_name: &str,
    ) -> Vec<TemplateBlockReferenceTarget> {
        let Some(owner) = self.block_reference_owner(template_file, block_name) else {
            return Vec::new();
        };

        let mut targets = Vec::new();
        self.collect_block_reference_targets(&owner, block_name, &mut targets);
        targets.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| position_cmp(a.range.start, b.range.start))
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        targets.dedup();
        targets
    }

    pub fn block_reference_owner(&self, template_file: &str, block_name: &str) -> Option<String> {
        let mut owner = self
            .template_declares_block(template_file, block_name)
            .then(|| template_file.to_string())?;
        let mut seen = HashSet::new();
        let mut cursor = template_file.to_string();

        while seen.insert(cursor.clone()) {
            let Some(parent_file) = self.templates.get(&cursor).and_then(|template| {
                template
                    .extends
                    .as_ref()
                    .filter(|parent| self.templates.contains_key(*parent))
                    .cloned()
            }) else {
                break;
            };
            if self.template_declares_block(&parent_file, block_name) {
                owner = parent_file.clone();
            }
            cursor = parent_file;
        }

        Some(owner)
    }

    pub fn collect_block_reference_targets(
        &self,
        template_file: &str,
        block_name: &str,
        targets: &mut Vec<TemplateBlockReferenceTarget>,
    ) {
        let Some(template) = self.templates.get(template_file) else {
            return;
        };
        targets.extend(
            template
                .blocks
                .iter()
                .filter(|block| block.name == block_name)
                .map(|block| TemplateBlockReferenceTarget {
                    path: template.path.clone(),
                    range: block.source_range,
                }),
        );
        if let Some(children) = self.children_by_parent.get(template_file) {
            for child in children {
                self.collect_block_reference_targets(child, block_name, targets);
            }
        }
    }

    pub fn template_declares_block(&self, template_file: &str, block_name: &str) -> bool {
        self.templates
            .get(template_file)
            .is_some_and(|template| template.blocks.iter().any(|block| block.name == block_name))
    }

    pub fn macro_reference_query(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateMacroReferenceQuery> {
        let template = self.templates.get(template_file)?;
        if let Some(occurrence) = template
            .macros
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
        {
            return Some(TemplateMacroReferenceQuery {
                target_template_file: template_file.to_string(),
                macro_name: occurrence.name.clone(),
                source_range: occurrence.source_range,
            });
        }
        template
            .macro_calls
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
            .map(|occurrence| TemplateMacroReferenceQuery {
                target_template_file: occurrence.target_template_file.clone(),
                macro_name: occurrence.macro_name.clone(),
                source_range: occurrence.source_range,
            })
    }

    pub fn macro_reference_targets(
        &self,
        target_template_file: &str,
        macro_name: &str,
    ) -> Vec<TemplateMacroReferenceTarget> {
        let mut targets = Vec::new();
        if let Some(target) = self.macro_definition_target(target_template_file, macro_name) {
            targets.push(target);
        }

        for template in self.templates.values() {
            targets.extend(
                template
                    .macro_calls
                    .iter()
                    .filter(|occurrence| {
                        occurrence.target_template_file == target_template_file
                            && occurrence.macro_name == macro_name
                    })
                    .map(|occurrence| TemplateMacroReferenceTarget {
                        path: template.path.clone(),
                        range: occurrence.source_range,
                    }),
            );
        }

        targets.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| position_cmp(a.range.start, b.range.start))
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        targets.dedup();
        targets
    }

    pub fn macro_definition_target(
        &self,
        target_template_file: &str,
        macro_name: &str,
    ) -> Option<TemplateMacroReferenceTarget> {
        let template = self.templates.get(target_template_file)?;
        template
            .macros
            .iter()
            .find(|occurrence| occurrence.name == macro_name)
            .map(|occurrence| TemplateMacroReferenceTarget {
                path: template.path.clone(),
                range: occurrence.source_range,
            })
    }

    pub fn dependency_names(&self, root_template: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        self.collect_dependency_names(root_template, &mut seen, &mut names);
        names
    }

    pub fn collect_dependency_names(
        &self,
        template_file: &str,
        seen: &mut HashSet<String>,
        names: &mut Vec<String>,
    ) {
        if !seen.insert(template_file.to_string()) {
            return;
        }
        let Some(template) = self.templates.get(template_file) else {
            return;
        };
        names.push(template_file.to_string());
        for dependency in &template.dependencies {
            self.collect_dependency_names(dependency, seen, names);
        }
    }

    pub fn semantic_tokens(&self, template_file: &str) -> Option<SemanticTokens> {
        let template = self.templates.get(template_file)?;
        let semantic = template.semantic.as_ref()?;
        Some(SemanticTokens {
            result_id: None,
            data: template_semantic_tokens(semantic, &template.content),
        })
    }

    pub fn semantic_definition(&self, template_file: &str, position: Position) -> Option<Location> {
        let template = self.templates.get(template_file)?;
        let offset = position_to_byte_offset(&template.content, position)?;
        let semantic = template.semantic.as_ref()?;
        let symbol = semantic.symbol_for_offset(offset)?;
        let span = symbol.span?;
        Some(Location {
            uri: Url::from_file_path(template.path.as_std_path()).ok()?,
            range: byte_range_to_lsp_range(
                &template.content,
                span.offset(),
                span.offset() + span.len(),
            ),
        })
    }

    pub fn semantic_references(&self, template_file: &str, position: Position) -> Vec<Location> {
        let Some(template) = self.templates.get(template_file) else {
            return Vec::new();
        };
        let Some(offset) = position_to_byte_offset(&template.content, position) else {
            return Vec::new();
        };
        let Some(semantic) = template.semantic.as_ref() else {
            return Vec::new();
        };
        let Some(symbol) = semantic.symbol_for_offset(offset) else {
            return Vec::new();
        };
        let Some(uri) = Url::from_file_path(template.path.as_std_path()).ok() else {
            return Vec::new();
        };

        let mut locations = Vec::new();
        if let Some(span) = symbol.span {
            locations.push(Location {
                uri: uri.clone(),
                range: byte_range_to_lsp_range(
                    &template.content,
                    span.offset(),
                    span.offset() + span.len(),
                ),
            });
        }
        locations.extend(
            semantic
                .references_to_symbol(symbol.id)
                .into_iter()
                .map(|reference| Location {
                    uri: uri.clone(),
                    range: byte_range_to_lsp_range(
                        &template.content,
                        reference.span.offset(),
                        reference.span.offset() + reference.span.len(),
                    ),
                }),
        );
        locations
    }
}

pub fn template_semantic_tokens(
    index: &TemplateSemanticIndex,
    content: &str,
) -> Vec<SemanticToken> {
    let mut spans = index.tokens.clone();
    spans.sort_by_key(|token| (token.span.offset(), token.span.len()));
    spans.dedup_by_key(|token| (token.span.offset(), token.span.len(), token.kind));

    let mut result = Vec::new();
    let mut previous_line = 0;
    let mut previous_start = 0;
    for token in spans {
        let start = token.span.offset();
        let end = start.saturating_add(token.span.len());
        if start >= content.len() || end > content.len() || content[start..end].contains('\n') {
            continue;
        }
        let (line, column) = byte_to_line_column(content, start);
        let line = line.saturating_sub(1);
        let start_character = column.saturating_sub(1);
        let delta_line = line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            start_character.saturating_sub(previous_start)
        } else {
            start_character
        };
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: content[start..end].chars().count() as u32,
            token_type: template_semantic_token_type(token.kind),
            token_modifiers_bitset: 0,
        });
        previous_line = line;
        previous_start = start_character;
    }
    result
}
pub fn position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
    let target_line = position.line as usize;
    let target_character = position.character as usize;
    let line_start = content
        .split_inclusive('\n')
        .take(target_line)
        .map(str::len)
        .sum::<usize>();
    let line = content.split_inclusive('\n').nth(target_line)?;
    let line_without_newline = line.strip_suffix('\n').unwrap_or(line);

    if target_character == 0 {
        return Some(line_start);
    }

    let mut chars_seen = 0;
    for (offset, _) in line_without_newline.char_indices() {
        if chars_seen == target_character {
            return Some(line_start + offset);
        }
        chars_seen += 1;
    }

    (chars_seen == target_character).then_some(line_start + line_without_newline.len())
}
pub fn template_semantic_token_type(kind: TemplateSemanticTokenKind) -> u32 {
    match kind {
        TemplateSemanticTokenKind::Variable => TEMPLATE_SEMANTIC_TOKEN_VARIABLE,
        TemplateSemanticTokenKind::Parameter => TEMPLATE_SEMANTIC_TOKEN_PARAMETER,
        TemplateSemanticTokenKind::Property => TEMPLATE_SEMANTIC_TOKEN_PROPERTY,
        TemplateSemanticTokenKind::Function => TEMPLATE_SEMANTIC_TOKEN_FUNCTION,
        TemplateSemanticTokenKind::Macro => TEMPLATE_SEMANTIC_TOKEN_MACRO,
        TemplateSemanticTokenKind::String => TEMPLATE_SEMANTIC_TOKEN_STRING,
        TemplateSemanticTokenKind::Number => TEMPLATE_SEMANTIC_TOKEN_NUMBER,
        TemplateSemanticTokenKind::Keyword => TEMPLATE_SEMANTIC_TOKEN_KEYWORD,
    }
}

pub const TEMPLATE_SEMANTIC_TOKEN_VARIABLE: u32 = 0;
pub const TEMPLATE_SEMANTIC_TOKEN_PARAMETER: u32 = 1;
pub const TEMPLATE_SEMANTIC_TOKEN_PROPERTY: u32 = 2;
pub const TEMPLATE_SEMANTIC_TOKEN_FUNCTION: u32 = 3;
pub const TEMPLATE_SEMANTIC_TOKEN_MACRO: u32 = 4;
pub const TEMPLATE_SEMANTIC_TOKEN_STRING: u32 = 5;
pub const TEMPLATE_SEMANTIC_TOKEN_NUMBER: u32 = 6;
pub const TEMPLATE_SEMANTIC_TOKEN_KEYWORD: u32 = 7;
