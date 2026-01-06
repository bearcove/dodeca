//! # bearmark
//!
//! A markdown rendering library with pluggable code block handlers.
//!
//! bearmark parses markdown documents and renders them to HTML, with support for:
//! - **Frontmatter**: TOML (`+++`) or YAML (`---`) frontmatter extraction
//! - **Headings**: Automatic extraction with slug generation for TOC
//! - **Requirement definitions**: `r[req.id]` syntax for specification traceability
//! - **Code blocks**: Pluggable handlers for syntax highlighting, diagrams, etc.
//! - **Link resolution**: `@/path` absolute links and relative link handling
//!
//! ## Example
//!
//! ```text
//! use bearmark::{render, RenderOptions};
//!
//! let markdown = "# Hello World\n\nSome content.";
//! let opts = RenderOptions::default();
//! let doc = render(markdown, &opts).await?;
//!
//! println!("HTML: {}", doc.html);
//! println!("Headings: {:?}", doc.headings);
//! ```

mod frontmatter;
mod handler;
mod handlers;
mod headings;
mod links;
mod render;
mod reqs;

pub use frontmatter::{Frontmatter, FrontmatterFormat, parse_frontmatter, strip_frontmatter};
pub use handler::{BoxedHandler, BoxedReqHandler, CodeBlockHandler, DefaultReqHandler, ReqHandler};
pub use headings::{Heading, slugify};
pub use links::resolve_link;
pub use render::{DocElement, Document, Paragraph, RenderOptions, render};
pub use reqs::{
    ExtractedReqs, ReqDefinition, ReqLevel, ReqMetadata, ReqStatus, ReqWarning, ReqWarningKind,
    Rfc2119Keyword, SourceSpan, detect_rfc2119_keywords, extract_reqs_with_warnings,
};

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

    /// Duplicate requirement ID found
    #[error("duplicate requirement ID: {0}")]
    DuplicateReq(String),

    /// Code block handler failed
    #[error("code block handler error for language '{language}': {message}")]
    CodeBlockHandler { language: String, message: String },
}

/// Result type alias for bearmark operations.
pub type Result<T> = std::result::Result<T, Error>;
