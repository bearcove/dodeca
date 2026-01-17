//! Code block handler trait and utilities.
//!
//! This module provides the [`CodeBlockHandler`] trait for implementing
//! custom code block rendering (syntax highlighting, diagram rendering, etc.)

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::Result;
use crate::reqs::ReqDefinition;

/// A handler for rendering code blocks.
///
/// Implementations can provide syntax highlighting, diagram rendering,
/// or any other transformation of code block content.
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

/// A handler for rendering req definitions.
///
/// Reqs are rendered with opening and closing HTML, allowing the req content
/// (paragraphs, code blocks, etc.) to be rendered in between.
pub trait ReqHandler: Send + Sync {
    /// Render the opening HTML for a req definition.
    ///
    /// This is called when a req is first detected. The returned HTML should
    /// contain the opening tags that will wrap the req content.
    ///
    /// # Returns
    /// The opening HTML string (e.g., `<div class="req" id="r-my.req">`).
    fn start<'a>(
        &'a self,
        req: &'a ReqDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    /// Render the closing HTML for a req definition.
    ///
    /// This is called when the req content is finished. The returned HTML
    /// should close any tags opened by `start`.
    ///
    /// # Returns
    /// The closing HTML string (e.g., `</div>`).
    fn end<'a>(
        &'a self,
        req: &'a ReqDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
}

/// Type alias for a boxed req handler.
pub type BoxedReqHandler = Arc<dyn ReqHandler>;

// @tracey:ignore-start
/// A handler for rendering inline code spans.
///
/// This allows customizing how inline `code` is rendered, for example
/// to transform `r[rule.id]` references into clickable links.
// @tracey:ignore-end
pub trait InlineCodeHandler: Send + Sync {
    /// Render an inline code span to HTML.
    ///
    /// # Arguments
    /// * `code` - The code content (without backticks)
    ///
    /// # Returns
    /// The rendered HTML string. Return `None` to use the default rendering.
    fn render(&self, code: &str) -> Option<String>;
}

/// Type alias for a boxed inline code handler.
pub type BoxedInlineCodeHandler = Arc<dyn InlineCodeHandler>;

/// A handler for resolving internal links.
///
/// This allows the caller to provide custom link resolution logic,
/// including dependency tracking for incremental rebuilds.
///
pub trait LinkResolver: Send + Sync {
    /// Resolve a link to its final URL.
    ///
    /// # Arguments
    /// * `link` - The raw link from the markdown (e.g., `@/guide/intro.md`)
    /// * `source_path` - The path of the source file containing the link
    ///
    /// # Returns
    /// * `Some(url)` - The resolved URL to use
    /// * `None` - Use the default link resolution logic
    fn resolve<'a>(
        &'a self,
        link: &'a str,
        source_path: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;
}

/// Type alias for a boxed link resolver.
pub type BoxedLinkResolver = Arc<dyn LinkResolver>;

/// Default req handler that renders simple anchor divs.
///
/// This is used when no custom req handler is registered.
pub struct DefaultReqHandler;

impl ReqHandler for DefaultReqHandler {
    fn start<'a>(
        &'a self,
        req: &'a ReqDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            Ok(format!(
                "<div class=\"req\" id=\"{}\"><a class=\"req-link\" href=\"#{}\" title=\"{}\"><span>{}</span></a>",
                req.anchor_id, req.anchor_id, req.id, req.id
            ))
        })
    }

    fn end<'a>(
        &'a self,
        _req: &'a ReqDefinition,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move { Ok("</div>".to_string()) })
    }
}

/// A simple handler that wraps code in `<div class=\"code-block\"><pre><code>` tags without processing.
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
            Ok(format!(
                "<div class=\"code-block\"><pre><code{}>{}</code></pre></div>",
                lang_class, escaped
            ))
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
            "<div class=\"code-block\"><pre><code class=\"language-rust\">fn main() {}</code></pre></div>"
        );
    }

    #[tokio::test]
    async fn test_raw_code_handler_escapes_html() {
        let handler = RawCodeHandler;
        let result = handler.render("html", "<div>test</div>").await.unwrap();
        assert!(result.contains("&lt;div&gt;"));
    }
}
