//! svag - An SVG minifier
//!
//! svag optimizes SVG files while maintaining visual fidelity.

mod ast;
mod error;
mod optimize;
mod parse;
mod path;
mod serialize;

pub use ast::*;
pub use error::*;
pub use optimize::*;
pub use parse::*;
pub use serialize::*;

/// Minify an SVG string with default settings.
pub fn minify(svg: &str) -> Result<String, SvagError> {
    minify_with_options(svg, &Options::default())
}

/// Minify an SVG string with custom options.
pub fn minify_with_options(svg: &str, options: &Options) -> Result<String, SvagError> {
    let mut doc = parse_svg(svg)?;
    optimize(&mut doc, options);
    Ok(serialize(&doc, options))
}

/// Minification options.
#[derive(Debug, Clone)]
pub struct Options {
    /// Number of decimal places for coordinates (default: 2)
    pub precision: u8,
    /// Remove comments
    pub remove_comments: bool,
    /// Remove metadata elements
    pub remove_metadata: bool,
    /// Remove XML declaration
    pub remove_xml_declaration: bool,
    /// Remove DOCTYPE
    pub remove_doctype: bool,
    /// Remove unused namespaces
    pub remove_unused_namespaces: bool,
    /// Collapse unnecessary groups
    pub collapse_groups: bool,
    /// Remove hidden elements
    pub remove_hidden: bool,
    /// Remove empty containers
    pub remove_empty: bool,
    /// Minify colors (#ffffff -> #fff)
    pub minify_colors: bool,
    /// Remove default attribute values
    pub remove_defaults: bool,
    /// Minify path data
    pub minify_paths: bool,
    /// Minify styles
    pub minify_styles: bool,
    /// Merge adjacent paths with same attributes
    pub merge_paths: bool,
    /// Sort attributes for better gzip
    pub sort_attrs: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            precision: 2,
            remove_comments: true,
            remove_metadata: true,
            remove_xml_declaration: true,
            remove_doctype: true,
            remove_unused_namespaces: true,
            collapse_groups: true,
            remove_hidden: true,
            remove_empty: true,
            minify_colors: true,
            remove_defaults: true,
            minify_paths: true,
            minify_styles: true,
            merge_paths: false, // conservative default - can break things
            sort_attrs: true,
        }
    }
}
