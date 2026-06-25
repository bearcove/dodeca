//! Template authoring analysis: the index of blocks/macros/includes/route refs
//! and template-file diagnostics, plus frontmatter document targets. Lifted out
//! of `dodeca-authoring-lsp` so it can back picante tracked queries.
//!
//! These types embed `lsp_types::Range` (vendored to derive `Facet`); a protocol
//! type in the engine is a known layering smell, accepted to avoid migrating the
//! whole subsystem to a neutral range type. The editor crate re-exports
//! everything here, so its call sites are unchanged.

use std::cmp::Ordering;

use camino::Utf8PathBuf;
use eyre::{Result, eyre};
use gingembre::ast::{Ident, StringLit};
use lsp_types::{Position, Range};
use url::Url;

use crate::authoring_graph::byte_to_line_column;

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
