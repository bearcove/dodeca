//! Code block handler trait and utilities.
//!
//! This module provides the [`CodeBlockHandler`] trait for implementing
//! custom code block rendering (syntax highlighting, diagram rendering, etc.)

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::Result;
use crate::rules::RuleDefinition;

/// A handler for rendering code blocks.
///
/// Implementations can provide syntax highlighting, diagram rendering,
/// or any other transformation of code block content.
///
/// # Example
///
/// ```rust,ignore
/// use bearmark::{CodeBlockHandler, Result};
///
/// struct ArboriumHandler;
///
/// impl CodeBlockHandler for ArboriumHandler {
///     fn render<'a>(
///         &'a self,
///         language: &'a str,
///         code: &'a str,
///     ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
///         Box::pin(async move {
///             // Use arborium to highlight
///             Ok(arborium::highlight(language, code))
///         })
///     }
/// }
/// ```
pub trait CodeBlockHandler: Send + Sync {
    /// Render a code block to HTML.
    ///
    /// # Arguments
    /// * `language` - The language identifier (e.g., "rust", "python", "aa", "pik")
    /// * `code` - The raw code content
    ///
    /// # Returns
    /// The rendered HTML string, or an error if rendering fails.
    fn render<'a>(
        &'a self,
        language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
}

/// Type alias for a boxed code block handler.
pub type BoxedHandler = Arc<dyn CodeBlockHandler>;

/// A handler for rendering rule definitions.
///
/// Implementations can provide custom rule rendering with additional context
/// such as coverage status, implementation references, etc.
///
/// # Example
///
/// ```rust,ignore
/// use bearmark::{RuleHandler, RuleDefinition, Result};
///
/// struct TraceyRuleHandler {
///     coverage: Arc<RuleCoverage>,
/// }
///
/// impl RuleHandler for TraceyRuleHandler {
///     fn render<'a>(
///         &'a self,
///         rule: &'a RuleDefinition,
///     ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
///         Box::pin(async move {
///             let status = self.coverage.get(&rule.id);
///             // Render with covered/uncovered class, impl/verify links, etc.
///             Ok(format!("<div class=\"rule {}\" id=\"{}\">...</div>",
///                 if status.is_covered() { "covered" } else { "uncovered" },
///                 rule.anchor_id))
///         })
///     }
/// }
/// ```
pub trait RuleHandler: Send + Sync {
    /// Render a rule definition to HTML.
    ///
    /// # Arguments
    /// * `rule` - The rule definition containing id and anchor_id
    ///
    /// # Returns
    /// The rendered HTML string, or an error if rendering fails.
    fn render<'a>(
        &'a self,
        rule: &'a RuleDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
}

/// Type alias for a boxed rule handler.
pub type BoxedRuleHandler = Arc<dyn RuleHandler>;

/// Default rule handler that renders simple anchor divs.
///
/// This is used when no custom rule handler is registered.
pub struct DefaultRuleHandler;

impl RuleHandler for DefaultRuleHandler {
    fn render<'a>(
        &'a self,
        rule: &'a RuleDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            // Insert <wbr> after dots for better line breaking in narrow displays
            let display_id = rule.id.replace('.', ".<wbr>");

            Ok(format!(
                "<div class=\"rule\" id=\"{}\"><a class=\"rule-link\" href=\"#{}\" title=\"{}\"><span>[{}]</span></a></div>",
                rule.anchor_id, rule.anchor_id, rule.id, display_id
            ))
        })
    }
}

/// A simple handler that wraps code in `<pre><code>` tags without processing.
///
/// This is used as a fallback when no handler is registered for a language.
pub struct RawCodeHandler;

impl CodeBlockHandler for RawCodeHandler {
    fn render<'a>(
        &'a self,
        language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let escaped = html_escape(code);
            let lang_class = if language.is_empty() {
                String::new()
            } else {
                format!(" class=\"language-{}\"", html_escape(language))
            };
            Ok(format!("<pre><code{}>{}</code></pre>", lang_class, escaped))
        })
    }
}

/// Escape HTML special characters.
pub(crate) fn html_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }

    #[tokio::test]
    async fn test_raw_code_handler() {
        let handler = RawCodeHandler;
        let result = handler.render("rust", "fn main() {}").await.unwrap();
        assert_eq!(
            result,
            "<pre><code class=\"language-rust\">fn main() {}</code></pre>"
        );
    }

    #[tokio::test]
    async fn test_raw_code_handler_escapes_html() {
        let handler = RawCodeHandler;
        let result = handler.render("html", "<div>test</div>").await.unwrap();
        assert!(result.contains("&lt;div&gt;"));
    }
}
