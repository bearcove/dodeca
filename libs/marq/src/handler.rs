//! Code block handler trait and utilities.
//!
//! This module provides the [`CodeBlockHandler`] trait for implementing
//! custom code block rendering (syntax highlighting, diagram rendering, etc.)

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::Result;
use crate::reqs::ReqDefinition;

/// An HTML snippet to inject into the page's `<head>` (or body end).
///
/// Multiple handlers can request injections; they are deduplicated by `key`
/// so that e.g. the Mermaid.js loader script is only included once even if
/// multiple mermaid code blocks appear in a document.
pub struct HeadInjection {
    /// Unique key for deduplication (e.g., "mermaid").
    pub key: String,
    /// HTML to inject (e.g., a `<script>` or `<link>` tag).
    pub html: String,
}

/// The output of a code block handler.
///
/// Contains the rendered HTML that replaces the code block, plus optional
/// [`HeadInjection`]s that the caller should include in the page.
pub struct CodeBlockOutput {
    /// HTML where the code block appeared.
    pub html: String,
    /// Additional page resources (scripts, stylesheets, etc.).
    pub head_injections: Vec<HeadInjection>,
}

impl From<String> for CodeBlockOutput {
    fn from(html: String) -> Self {
        Self {
            html,
            head_injections: vec![],
        }
    }
}

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
    /// A [`CodeBlockOutput`] containing the rendered HTML and any head injections,
    /// or an error if rendering fails.
    fn render<'a>(
        &'a self,
        language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>>;
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

/// A wiki-style link parsed from `[[target]]` or `[[target|label]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The target text before the optional `|`.
    pub target: String,
}

/// Render instructions for a wiki-style link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLinkOutput {
    /// The href attribute to emit.
    pub href: String,
    /// Additional attributes to emit on the `<a>` element.
    pub attrs: Vec<(String, String)>,
}

impl WikiLinkOutput {
    /// Create a wiki link output with just an href.
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            attrs: Vec::new(),
        }
    }

    /// Add an attribute to the output.
    pub fn with_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.push((name.into(), value.into()));
        self
    }
}

/// A handler for resolving wiki-style links.
///
/// This allows callers to decide how `[[target]]` links become regular HTML
/// anchors while keeping the parsing of wiki-link syntax inside marq.
pub trait WikiLinkResolver: Send + Sync {
    /// Resolve a wiki-style link.
    ///
    /// # Arguments
    /// * `link` - The parsed wiki link target
    /// * `source_path` - The path of the source file containing the link
    ///
    /// # Returns
    /// * `Some(output)` - Render an `<a>` element using this output
    /// * `None` - Leave the wiki-link syntax as plain text
    fn resolve<'a>(
        &'a self,
        link: &'a WikiLink,
        source_path: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Option<WikiLinkOutput>> + Send + 'a>>;
}

/// Type alias for a boxed wiki link resolver.
pub type BoxedWikiLinkResolver = Arc<dyn WikiLinkResolver>;

/// Arguments passed to a shortcode, in the form they were written.
///
/// marq deliberately does not normalize these into a single data model: it keeps
/// no opinion (and no YAML dependency) about how arguments are typed. The resolver
/// — which owns the real data model and template engine — parses them. This is the
/// same passthrough discipline marq uses for links, and it is what keeps dependency
/// tracking correct: the resolver runs inside the host's tracked query, so any
/// template/asset it reads while interpreting these args is recorded as a dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcodeArgs {
    /// The raw YAML mapping value of a fenced `+++ :name: <yaml> +++` shortcode,
    /// i.e. everything nested under the `:name:` key (the key line itself removed).
    Yaml(String),
    /// Parenthesised `key=value` pairs of an inline/blockquote `*:name(k=v, ...)*`
    /// shortcode, parsed positionally in source order. Empty when no parens are given.
    Pairs(Vec<(String, String)>),
}

/// A shortcode invocation parsed from markdown.
///
/// Two grammars produce these, both detected in the event stream without lexer
/// changes (pulldown-cmark already emits the underlying events):
/// - fenced `+++ :name: <yaml> +++` → [`ShortcodeArgs::Yaml`], `body` is `None`;
/// - blockquote/inline `*:name(args)*` (+ optional blockquote body) →
///   [`ShortcodeArgs::Pairs`], `body` is the rendered HTML of the block body.
#[derive(Debug, Clone)]
pub struct Shortcode<'a> {
    /// Shortcode name, without the leading `:`.
    pub name: &'a str,
    /// Arguments, in the form they were written.
    pub args: &'a ShortcodeArgs,
    /// Rendered HTML of the shortcode body, if the grammar carries one.
    ///
    /// `Some` for blockquote body shortcodes (`> *:name*` followed by content);
    /// `None` for fenced shortcodes and bare inline `*:name*` with no body.
    pub body: Option<&'a str>,
}

/// The output of a shortcode resolver.
///
/// Mirrors [`CodeBlockOutput`]: rendered HTML plus optional [`HeadInjection`]s so a
/// shortcode can pull in a one-time script/stylesheet (e.g. a video embed loader).
pub struct ShortcodeOutput {
    /// HTML to splice in where the shortcode appeared.
    pub html: String,
    /// Additional page resources, deduplicated by key like code-block injections.
    pub head_injections: Vec<HeadInjection>,
}

impl From<String> for ShortcodeOutput {
    fn from(html: String) -> Self {
        Self {
            html,
            head_injections: vec![],
        }
    }
}

/// A handler for rendering shortcodes.
///
/// marq stays dependency-agnostic: it detects shortcode syntax and renders the body
/// markdown, then hands the invocation to this resolver. The default behavior (no
/// resolver registered) leaves the source untouched, so marq bakes nothing on its own.
pub trait ShortcodeResolver: Send + Sync {
    /// Resolve a shortcode invocation to HTML.
    ///
    /// # Returns
    /// * `Some(output)` — splice this HTML in place of the shortcode.
    /// * `None` — the resolver declines; marq falls back to leaving the source.
    fn resolve<'a>(
        &'a self,
        shortcode: Shortcode<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<ShortcodeOutput>>> + Send + 'a>>;
}

/// Type alias for a boxed shortcode resolver.
pub type BoxedShortcodeResolver = Arc<dyn ShortcodeResolver>;

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
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
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
            )
            .into())
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
        let output = handler.render("rust", "fn main() {}").await.unwrap();
        assert_eq!(
            output.html,
            "<div class=\"code-block\"><pre><code class=\"language-rust\">fn main() {}</code></pre></div>"
        );
        assert!(output.head_injections.is_empty());
    }

    #[tokio::test]
    async fn test_raw_code_handler_escapes_html() {
        let handler = RawCodeHandler;
        let output = handler.render("html", "<div>test</div>").await.unwrap();
        assert!(output.html.contains("&lt;div&gt;"));
        assert!(output.head_injections.is_empty());
    }
}
