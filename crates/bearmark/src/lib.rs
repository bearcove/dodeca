//! # bearmark
//!
//! A markdown rendering library with pluggable code block handlers.
//!
//! bearmark parses markdown documents and renders them to HTML, with support for:
//! - **Frontmatter**: TOML (`+++`) or YAML (`---`) frontmatter extraction
//! - **Headings**: Automatic extraction with slug generation for TOC
//! - **Rule definitions**: `r[rule.id]` syntax for specification traceability
//! - **Code blocks**: Pluggable handlers for syntax highlighting, diagrams, etc.
//! - **Link resolution**: `@/path` absolute links and relative link handling
//!
//! ## Example
//!
//! ```rust,ignore
//! use bearmark::{render, RenderOptions, Document};
//!
//! let markdown = r#"
//! +++
//! title = "My Document"
//! +++
//!
//! # Hello World
//!
//! r[my.rule]
//! This rule MUST be followed.
//!
//! ```rust
//! fn main() {}
//! ```
//! "#;
//!
//! let opts = RenderOptions::default();
//! let doc = render(markdown, &opts).await?;
//!
//! println!("Title: {:?}", doc.frontmatter.map(|f| f.title));
//! println!("Headings: {:?}", doc.headings);
//! println!("Rules: {:?}", doc.rules);
//! println!("HTML: {}", doc.html);
//! ```

mod frontmatter;
mod handler;
mod handlers;
mod headings;
mod links;
mod render;
mod rules;

pub use frontmatter::{Frontmatter, FrontmatterFormat, parse_frontmatter, strip_frontmatter};
pub use handler::{
    BoxedHandler, BoxedRuleHandler, CodeBlockHandler, DefaultRuleHandler, RuleHandler,
};
pub use headings::{Heading, slugify};
pub use links::resolve_link;
pub use render::{Document, RenderOptions, render};
pub use rules::RuleDefinition;

// Feature-gated handler exports
#[cfg(feature = "highlight")]
pub use handlers::ArboriumHandler;

#[cfg(feature = "aasvg")]
pub use handlers::AasvgHandler;

#[cfg(feature = "pikru")]
pub use handlers::PikruHandler;

/// Error type for bearmark operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Frontmatter parsing failed
    #[error("frontmatter parse error: {0}")]
    FrontmatterParse(String),

    /// Duplicate rule ID found
    #[error("duplicate rule ID: {0}")]
    DuplicateRule(String),

    /// Code block handler failed
    #[error("code block handler error for language '{language}': {message}")]
    CodeBlockHandler { language: String, message: String },
}

/// Result type alias for bearmark operations.
pub type Result<T> = std::result::Result<T, Error>;
