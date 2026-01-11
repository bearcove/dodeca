//! Error types for template parsing and evaluation
//!
//! Error types carry structured information for debugging.

#![allow(unused_assignments)]

use std::sync::Arc;
use thiserror::Error;

/// A span in source code (offset, length)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceSpan {
    offset: usize,
    len: usize,
}

impl SourceSpan {
    /// Create a new span from offset and length
    pub fn new(offset: usize, len: usize) -> Self {
        Self { offset, len }
    }

    /// Get the offset (start position)
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Get the length
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if the span is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// A template source file for error reporting
#[derive(Debug, Clone)]
pub struct TemplateSource {
    /// Name of the template (usually filename)
    pub name: String,
    /// The full source text
    pub source: Arc<String>,
}

impl TemplateSource {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: Arc::new(source.into()),
        }
    }

    /// Create a NamedSource for error reporting
    pub fn named_source(&self) -> NamedSource {
        NamedSource::new(self.name.clone(), (*self.source).clone())
    }
}

/// Named source for error reporting (simplified from miette)
#[derive(Debug, Clone)]
pub struct NamedSource {
    pub name: String,
    pub source: String,
}

impl NamedSource {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
}

/// All template errors
#[derive(Error, Debug)]
pub enum TemplateError {
    #[error("Syntax error: {0}")]
    Syntax(#[from] SyntaxError),

    #[error("Unknown field: {0}")]
    UnknownField(#[from] UnknownFieldError),

    #[error("Type error: {0}")]
    Type(#[from] TypeError),

    #[error("Undefined variable: {0}")]
    Undefined(#[from] UndefinedError),

    #[error("Unknown filter: {0}")]
    UnknownFilter(#[from] UnknownFilterError),

    #[error("Unknown test: {0}")]
    UnknownTest(#[from] UnknownTestError),
}

/// Syntax error during parsing
#[derive(Error, Debug)]
#[error("{}: Unexpected {found}, expected {expected}", src.name)]
pub struct SyntaxError {
    /// What we found
    pub found: String,
    /// What we expected
    pub expected: String,
    /// Location in source
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Unknown field access on a type
#[derive(Error, Debug)]
#[error("{}: Type `{base_type}` has no field `{field}` (available: {})", src.name, known_fields.join(", "))]
pub struct UnknownFieldError {
    /// The type being accessed
    pub base_type: String,
    /// The field that doesn't exist
    pub field: String,
    /// Known fields on this type
    pub known_fields: Vec<String>,
    /// Location of the field access
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Type error (e.g., iterating over non-iterable)
#[derive(Error, Debug)]
#[error("{}: Expected {expected}, found {found} ({context})", src.name)]
pub struct TypeError {
    /// What type was expected
    pub expected: String,
    /// What type was found
    pub found: String,
    /// Context for the error
    pub context: String,
    /// Location
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Undefined variable
#[derive(Error, Debug)]
#[error("{}: Variable `{name}` is not defined (available: {})", src.name, available.join(", "))]
pub struct UndefinedError {
    /// The undefined variable name
    pub name: String,
    /// Variables that are available in scope
    pub available: Vec<String>,
    /// Location
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Unknown filter
#[derive(Error, Debug)]
#[error("{}: Unknown filter `{name}` (available: {})", src.name, known_filters.join(", "))]
pub struct UnknownFilterError {
    /// The filter that doesn't exist
    pub name: String,
    /// Known filters
    pub known_filters: Vec<String>,
    /// Location
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Unknown test function
#[derive(Error, Debug)]
#[error("{}: Unknown test `{name}` (available: starting_with, ending_with, containing, defined, undefined, none, string, number, odd, even, empty)", src.name)]
pub struct UnknownTestError {
    /// The test that doesn't exist
    pub name: String,
    /// Location
    pub span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

/// Unclosed delimiter (tag, block, etc.)
#[derive(Error, Debug)]
#[error("{}: Unclosed {kind}, add `{close_delim}` to close", src.name)]
pub struct UnclosedError {
    /// What was left unclosed
    pub kind: String,
    /// The closing delimiter needed
    pub close_delim: String,
    /// Where it was opened
    pub open_span: SourceSpan,
    /// The source code
    pub src: NamedSource,
}

impl From<UnclosedError> for TemplateError {
    fn from(e: UnclosedError) -> Self {
        TemplateError::Syntax(SyntaxError {
            found: "end of input".to_string(),
            expected: e.close_delim.clone(),
            span: e.open_span,
            src: e.src,
        })
    }
}
