//! Error types with rich diagnostics using miette
//!
//! These errors carry source spans for beautiful error messages.

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

/// Source context for error reporting
#[derive(Debug, Clone)]
pub struct SourceContext {
    /// Name of the source (filename or "<input>")
    pub name: String,
    /// The full source text
    pub source: String,
}

impl SourceContext {
    /// Create a new source context
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }

    /// Create a NamedSource for miette
    pub fn named_source(&self) -> NamedSource<String> {
        NamedSource::new(&self.name, self.source.clone())
    }
}

// ============================================================================
// Parse Errors
// ============================================================================

/// Errors that occur during parsing
#[derive(Error, Diagnostic, Debug)]
pub enum ParseError {
    #[error("unexpected token")]
    #[diagnostic(code(pikru::parse::unexpected_token))]
    UnexpectedToken {
        #[source_code]
        src: NamedSource<String>,
        #[label("found this")]
        span: SourceSpan,
        expected: String,
    },

    #[error("unterminated string")]
    #[diagnostic(code(pikru::parse::unterminated_string))]
    UnterminatedString {
        #[source_code]
        src: NamedSource<String>,
        #[label("string starts here")]
        span: SourceSpan,
    },

    #[error("invalid number: {message}")]
    #[diagnostic(code(pikru::parse::invalid_number))]
    InvalidNumber {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("invalid number")]
        span: SourceSpan,
    },

    #[error("unknown keyword: {keyword}")]
    #[diagnostic(code(pikru::parse::unknown_keyword))]
    UnknownKeyword {
        keyword: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("unknown keyword")]
        span: SourceSpan,
    },
}

// ============================================================================
// Evaluation Errors
// ============================================================================

/// Errors that occur during expression evaluation
#[derive(Error, Diagnostic, Debug)]
pub enum EvalError {
    #[error("undefined variable: {name}")]
    #[diagnostic(code(pikru::eval::undefined_variable))]
    UndefinedVariable {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not defined")]
        span: SourceSpan,
        #[help]
        suggestion: Option<String>,
    },

    #[error("unknown object: {name}")]
    #[diagnostic(code(pikru::eval::unknown_object))]
    UnknownObject {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not found")]
        span: SourceSpan,
        #[help]
        suggestion: Option<String>,
    },

    #[error("cannot add two positions")]
    #[diagnostic(
        code(pikru::eval::cannot_add_positions),
        help("use `pos - pos` to get displacement, or `pos + offset` to translate")
    )]
    CannotAddPositions {
        #[source_code]
        src: NamedSource<String>,
        #[label("this is a position")]
        lhs: SourceSpan,
        #[label("this is also a position")]
        rhs: SourceSpan,
    },

    #[error("type mismatch: expected {expected}, got {got}")]
    #[diagnostic(code(pikru::eval::type_mismatch))]
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
        #[source_code]
        src: NamedSource<String>,
        #[label("this expression has type {got}")]
        span: SourceSpan,
    },

    #[error("division by zero")]
    #[diagnostic(code(pikru::eval::division_by_zero))]
    DivisionByZero {
        #[source_code]
        src: NamedSource<String>,
        #[label("divisor is zero")]
        span: SourceSpan,
    },

    #[error("sqrt of negative number")]
    #[diagnostic(code(pikru::eval::sqrt_negative))]
    SqrtNegative {
        #[source_code]
        src: NamedSource<String>,
        #[label("this value is negative")]
        span: SourceSpan,
    },

    #[error("ordinal out of range: {ordinal}")]
    #[diagnostic(
        code(pikru::eval::ordinal_out_of_range),
        help("only {count} objects of this type exist")
    )]
    OrdinalOutOfRange {
        ordinal: u32,
        count: usize,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such object")]
        span: SourceSpan,
    },

    #[error("invalid numeric value")]
    #[diagnostic(code(pikru::eval::invalid_numeric))]
    InvalidNumeric {
        #[source_code]
        src: NamedSource<String>,
        #[label("this value is NaN or infinite")]
        span: SourceSpan,
    },

    #[error("no previous object")]
    #[diagnostic(
        code(pikru::eval::no_previous),
        help("create at least one object before using 'previous'")
    )]
    NoPrevious {
        #[source_code]
        src: NamedSource<String>,
        #[label("no previous object exists")]
        span: SourceSpan,
    },

    #[error("cannot reference 'this' outside object definition")]
    #[diagnostic(code(pikru::eval::no_this))]
    NoThis {
        #[source_code]
        src: NamedSource<String>,
        #[label("'this' not available here")]
        span: SourceSpan,
    },
}

// ============================================================================
// Render Errors
// ============================================================================

/// Errors that occur during rendering
#[derive(Error, Diagnostic, Debug)]
pub enum RenderError {
    #[error("invalid scale: {value}")]
    #[diagnostic(code(pikru::render::invalid_scale))]
    InvalidScale { value: f64 },

    #[error("empty diagram")]
    #[diagnostic(code(pikru::render::empty_diagram))]
    EmptyDiagram,

    #[error("infinite or NaN in bounds")]
    #[diagnostic(code(pikru::render::invalid_bounds))]
    InvalidBounds,
}

// ============================================================================
// User-Facing Errors
// ============================================================================

/// Errors from the `error` statement in pikchr
#[derive(Error, Diagnostic, Debug)]
#[error("{message}")]
#[diagnostic(code(pikru::user_error))]
pub struct UserError {
    pub message: String,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("error raised here")]
    pub span: SourceSpan,
}

/// Assertion failure from the `assert` statement
#[derive(Error, Diagnostic, Debug)]
#[error("assertion failed")]
#[diagnostic(code(pikru::assertion_failed))]
pub struct AssertionError {
    #[source_code]
    pub src: NamedSource<String>,
    #[label("assertion failed here")]
    pub span: SourceSpan,
    #[help]
    pub details: Option<String>,
}
