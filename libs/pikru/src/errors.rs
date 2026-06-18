//! Error types with rich diagnostics using ariadne
//!
//! These errors carry source spans for beautiful error messages.

use crate::types::Span;
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
}

// ============================================================================
// Parse Errors
// ============================================================================

/// Errors that occur during parsing
#[derive(Error, Debug)]
pub enum ParseError {
    #[error("unexpected token: expected {expected}")]
    UnexpectedToken { span: Span, expected: String },

    #[error("unterminated string")]
    UnterminatedString { span: Span },

    #[error("invalid number: {message}")]
    InvalidNumber { message: String, span: Span },

    #[error("unknown keyword: {keyword}")]
    UnknownKeyword { keyword: String, span: Span },
}

// ============================================================================
// Evaluation Errors
// ============================================================================

/// Errors that occur during expression evaluation
#[derive(Error, Debug)]
pub enum EvalError {
    #[error("undefined variable: {name}")]
    UndefinedVariable {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },

    #[error("unknown object: {name}")]
    UnknownObject {
        name: String,
        span: Span,
        suggestion: Option<String>,
    },

    #[error("cannot add two positions")]
    CannotAddPositions { lhs: Span, rhs: Span },

    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
        span: Span,
    },

    #[error("division by zero")]
    DivisionByZero { span: Span },

    #[error("sqrt of negative number")]
    SqrtNegative { span: Span },

    #[error("ordinal out of range: {ordinal} (only {count} objects exist)")]
    OrdinalOutOfRange {
        ordinal: u32,
        count: usize,
        span: Span,
    },

    #[error("invalid numeric value (NaN or infinite)")]
    InvalidNumeric { span: Span },

    #[error("no previous object")]
    NoPrevious { span: Span },

    #[error("cannot reference 'this' outside object definition")]
    NoThis { span: Span },
}

// ============================================================================
// Render Errors
// ============================================================================

/// Errors that occur during rendering
#[derive(Error, Debug)]
pub enum RenderError {
    #[error("invalid scale: {value}")]
    InvalidScale { value: f64 },

    #[error("empty diagram")]
    EmptyDiagram,

    #[error("infinite or NaN in bounds")]
    InvalidBounds,
}

// ============================================================================
// User-Facing Errors
// ============================================================================

/// Errors from the `error` statement in pikchr
#[derive(Error, Debug)]
#[error("{message}")]
pub struct UserError {
    pub message: String,
    pub span: Span,
}

/// Assertion failure from the `assert` statement
#[derive(Error, Debug)]
#[error("assertion failed")]
pub struct AssertionError {
    pub span: Span,
    pub details: Option<String>,
}

// ============================================================================
// Unified Error Type
// ============================================================================

/// Main error type for pikru operations
#[derive(Error, Debug)]
pub enum PikruError {
    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error(transparent)]
    Eval(#[from] EvalError),

    #[error(transparent)]
    Render(#[from] RenderError),

    #[error(transparent)]
    User(#[from] UserError),

    #[error(transparent)]
    Assertion(#[from] AssertionError),

    #[error("{0}")]
    Generic(String),
}

impl PikruError {
    /// Convert the error to an ariadne report
    pub fn to_report(&self, source_name: &str, source: &str) -> String {
        use ariadne::{Color, Label, Report, ReportKind, Source};
        use std::ops::Range;

        // Helper to convert Span to Range<usize> with source ID
        let to_range =
            |span: &Span| -> (&str, Range<usize>) { (source_name, span.start..span.end) };

        let mut output = Vec::new();

        let report = match self {
            PikruError::Parse(e) => match e {
                ParseError::UnexpectedToken { span, expected } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message("unexpected token".to_string())
                        .with_label(
                            Label::new(to_range(span))
                                .with_message(format!("expected {}", expected))
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                ParseError::UnterminatedString { span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message("unterminated string")
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("string starts here")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                ParseError::InvalidNumber { message, span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message(format!("invalid number: {}", message))
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("invalid number")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                ParseError::UnknownKeyword { keyword, span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message(format!("unknown keyword: {}", keyword))
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("unknown keyword")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
            },
            PikruError::Eval(e) => match e {
                EvalError::UndefinedVariable {
                    name,
                    span,
                    suggestion,
                } => {
                    let mut report = Report::build(ReportKind::Error, to_range(span))
                        .with_message(format!("undefined variable: {}", name))
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("not defined")
                                .with_color(Color::Red),
                        );
                    if let Some(sugg) = suggestion {
                        report = report.with_help(sugg.clone());
                    }
                    report.finish()
                }
                EvalError::UnknownObject {
                    name,
                    span,
                    suggestion,
                } => {
                    let mut report = Report::build(ReportKind::Error, to_range(span))
                        .with_message(format!("unknown object: {}", name))
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("not found")
                                .with_color(Color::Red),
                        );
                    if let Some(sugg) = suggestion {
                        report = report.with_help(sugg.clone());
                    }
                    report.finish()
                }
                EvalError::CannotAddPositions { lhs, rhs } => {
                    Report::build(ReportKind::Error, to_range(lhs))
                        .with_message("cannot add two positions")
                        .with_label(
                            Label::new(to_range(lhs))
                                .with_message("this is a position")
                                .with_color(Color::Red),
                        )
                        .with_label(
                            Label::new(to_range(rhs))
                                .with_message("this is also a position")
                                .with_color(Color::Red),
                        )
                        .with_help(
                            "use `pos - pos` to get displacement, or `pos + offset` to translate",
                        )
                        .finish()
                }
                EvalError::TypeMismatch {
                    expected,
                    got,
                    span,
                } => Report::build(ReportKind::Error, to_range(span))
                    .with_message(format!("type mismatch: expected {}, got {}", expected, got))
                    .with_label(
                        Label::new(to_range(span))
                            .with_message(format!("this expression has type {}", got))
                            .with_color(Color::Red),
                    )
                    .finish(),
                EvalError::DivisionByZero { span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message("division by zero")
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("divisor is zero")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                EvalError::SqrtNegative { span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message("sqrt of negative number")
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("this value is negative")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                EvalError::OrdinalOutOfRange {
                    ordinal,
                    count,
                    span,
                } => Report::build(ReportKind::Error, to_range(span))
                    .with_message(format!("ordinal out of range: {}", ordinal))
                    .with_label(
                        Label::new(to_range(span))
                            .with_message("no such object")
                            .with_color(Color::Red),
                    )
                    .with_help(format!("only {} objects of this type exist", count))
                    .finish(),
                EvalError::InvalidNumeric { span } => {
                    Report::build(ReportKind::Error, to_range(span))
                        .with_message("invalid numeric value")
                        .with_label(
                            Label::new(to_range(span))
                                .with_message("this value is NaN or infinite")
                                .with_color(Color::Red),
                        )
                        .finish()
                }
                EvalError::NoPrevious { span } => Report::build(ReportKind::Error, to_range(span))
                    .with_message("no previous object")
                    .with_label(
                        Label::new(to_range(span))
                            .with_message("no previous object exists")
                            .with_color(Color::Red),
                    )
                    .with_help("create at least one object before using 'previous'")
                    .finish(),
                EvalError::NoThis { span } => Report::build(ReportKind::Error, to_range(span))
                    .with_message("cannot reference 'this' outside object definition")
                    .with_label(
                        Label::new(to_range(span))
                            .with_message("'this' not available here")
                            .with_color(Color::Red),
                    )
                    .finish(),
            },
            PikruError::Render(e) => Report::build(ReportKind::Error, (source_name, 0..0))
                .with_message(e.to_string())
                .finish(),
            PikruError::User(e) => Report::build(ReportKind::Error, to_range(&e.span))
                .with_message(&e.message)
                .with_label(
                    Label::new(to_range(&e.span))
                        .with_message("error raised here")
                        .with_color(Color::Red),
                )
                .finish(),
            PikruError::Assertion(e) => {
                let mut report = Report::build(ReportKind::Error, to_range(&e.span))
                    .with_message("assertion failed")
                    .with_label(
                        Label::new(to_range(&e.span))
                            .with_message("assertion failed here")
                            .with_color(Color::Red),
                    );
                if let Some(details) = &e.details {
                    report = report.with_help(details.clone());
                }
                report.finish()
            }
            PikruError::Generic(msg) => Report::build(ReportKind::Error, (source_name, 0..0))
                .with_message(msg)
                .finish(),
        };

        report
            .write((source_name, Source::from(source)), &mut output)
            .unwrap();
        String::from_utf8(output).unwrap()
    }
}
