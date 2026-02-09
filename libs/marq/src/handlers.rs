//! Built-in code block handlers.
//!
//! Some handlers are available when their respective feature flags are enabled:
//! - `highlight` - Syntax highlighting via arborium
//! - `aasvg` - ASCII art to SVG conversion
//! - `pikru` - Pikchr diagram rendering
//!
//! The following handlers are always available:
//! - `TermHandler` - Terminal output passthrough
//! - `MermaidHandler` - Client-side Mermaid.js diagrams

use std::future::Future;
use std::pin::Pin;

use crate::Result;
use crate::handler::{CodeBlockHandler, CodeBlockOutput};

/// Syntax highlighting handler using arborium.
///
/// Requires the `highlight` feature.
#[cfg(feature = "highlight")]
pub struct ArboriumHandler {
    highlighter: std::sync::Mutex<arborium::Highlighter>,
    /// Whether to show a language header above code blocks
    show_language_header: bool,
}

#[cfg(feature = "highlight")]
impl ArboriumHandler {
    /// Create a new ArboriumHandler with default config.
    pub fn new() -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::new()),
            show_language_header: true,
        }
    }

    /// Create a new ArboriumHandler with custom config.
    pub fn with_config(config: arborium::Config) -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::with_config(config)),
            show_language_header: true,
        }
    }

    /// Enable or disable the language header above code blocks.
    ///
    /// When enabled, code blocks will be wrapped in a container with a header
    /// showing the language name, similar to the compare block style.
    pub fn with_language_header(mut self, show: bool) -> Self {
        self.show_language_header = show;
        self
    }
}

#[cfg(feature = "highlight")]
impl Default for ArboriumHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "highlight")]
impl CodeBlockHandler for ArboriumHandler {
    fn render<'a>(
        &'a self,
        language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            use crate::handler::html_escape;

            // Empty language means no syntax highlighting requested - render as plain
            if language.is_empty() {
                let escaped = html_escape(code);
                return Ok(format!(
                    "<div class=\"code-block\"><pre><code>{escaped}</code></pre></div>"
                )
                .into());
            }

            // Map common language aliases to arborium language names
            let arborium_lang = match language {
                "jinja" => "jinja2",
                _ => language,
            };

            let escaped_lang = html_escape(language);

            // Try to highlight with arborium
            let mut hl = self.highlighter.lock().unwrap();
            let highlighted_code = match hl.highlight(arborium_lang, code) {
                Ok(html) => {
                    // Trim trailing newline from arborium output
                    // See: https://github.com/bearcove/arborium/issues/128
                    html.trim_end_matches('\n').to_string()
                }
                Err(_e) => {
                    // Fall back to plain text rendering for unsupported languages
                    html_escape(code)
                }
            };

            // Build the output with data-lang for CSS targeting
            if self.show_language_header {
                Ok(format!(
                    "<div class=\"code-block\" data-lang=\"{escaped_lang}\"><div class=\"code-header\">{escaped_lang}</div><pre><code class=\"language-{escaped_lang}\">{highlighted_code}</code></pre></div>"
                )
                .into())
            } else {
                Ok(format!(
                    "<div class=\"code-block\" data-lang=\"{escaped_lang}\"><pre><code class=\"language-{escaped_lang}\">{highlighted_code}</code></pre></div>"
                )
                .into())
            }
        })
    }
}

/// Terminal output handler that passes through HTML without escaping.
///
/// This handler is designed for pre-rendered terminal output from tools like
/// `ddc term` which produce HTML with `<t-*>` custom elements for styled text.
/// The content is wrapped in a code block container but not HTML-escaped.
pub struct TermHandler;

impl TermHandler {
    /// Create a new TermHandler.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TermHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeBlockHandler for TermHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            // Pass through the HTML without escaping - it's already valid HTML
            // from the terminal renderer (contains <t-b>, <t-f>, etc. elements)
            Ok(format!(
                "<div class=\"code-block term-output\"><pre><code>{}</code></pre></div>",
                code
            )
            .into())
        })
    }
}

/// ASCII art to SVG handler using aasvg.
///
/// Requires the `aasvg` feature.
#[cfg(feature = "aasvg")]
pub struct AasvgHandler;

#[cfg(feature = "aasvg")]
impl AasvgHandler {
    /// Create a new AasvgHandler.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "aasvg")]
impl Default for AasvgHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "aasvg")]
impl CodeBlockHandler for AasvgHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            let svg = aasvg::render(code);
            Ok(svg.into())
        })
    }
}

/// Pikchr diagram handler using pikru.
///
/// Requires the `pikru` feature.
#[cfg(feature = "pikru")]
pub struct PikruHandler {
    /// Whether to use CSS variables for colors (for dark mode support)
    pub css_variables: bool,
}

#[cfg(feature = "pikru")]
impl PikruHandler {
    /// Create a new PikruHandler.
    pub fn new() -> Self {
        Self {
            css_variables: false,
        }
    }

    /// Create a new PikruHandler with CSS variable support.
    pub fn with_css_variables(css_variables: bool) -> Self {
        Self { css_variables }
    }
}

#[cfg(feature = "pikru")]
impl Default for PikruHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "pikru")]
impl CodeBlockHandler for PikruHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            // Parse the pikchr source
            let program = match pikru::parse::parse(code) {
                Ok(p) => p,
                Err(e) => {
                    return Err(crate::Error::CodeBlockHandler {
                        language: "pik".to_string(),
                        message: format!("parse error: {}", e),
                    });
                }
            };

            // Expand macros
            let program = match pikru::macros::expand_macros(program) {
                Ok(p) => p,
                Err(e) => {
                    return Err(crate::Error::CodeBlockHandler {
                        language: "pik".to_string(),
                        message: format!("macro error: {}", e),
                    });
                }
            };

            // Render to SVG
            let options = pikru::render::RenderOptions {
                css_variables: self.css_variables,
            };
            match pikru::render::render_with_options(&program, &options) {
                Ok(svg) => Ok(svg.into()),
                Err(e) => Err(crate::Error::CodeBlockHandler {
                    language: "pik".to_string(),
                    message: format!("render error: {}", e),
                }),
            }
        })
    }
}

/// Mermaid diagram handler.
///
/// Emits a `<pre class="mermaid">` block for client-side rendering by
/// Mermaid.js, wrapped in `data-hotmeal-opaque` for live-reload compatibility.
/// Includes a head injection that loads Mermaid.js from CDN and listens for
/// `hotmeal:opaque-changed` events to re-render after live-reload patches.
pub struct MermaidHandler;

impl MermaidHandler {
    /// Create a new MermaidHandler.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MermaidHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeBlockHandler for MermaidHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            use crate::handler::{HeadInjection, html_escape};

            let escaped = html_escape(code);
            let html = format!(
                "<div data-hotmeal-opaque=\"mermaid\"><pre class=\"mermaid\">{escaped}</pre></div>"
            );

            let script = r#"<script type="module">
import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs';

function mermaidTheme() {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'default';
}

async function renderMermaidNode(pre) {
  pre.removeAttribute('data-processed');
  pre.innerHTML = pre.dataset.mermaidSource;
  mermaid.initialize({ startOnLoad: false, theme: mermaidTheme() });
  await mermaid.run({ nodes: [pre] });
}

async function reinitAllMermaid() {
  const nodes = document.querySelectorAll('pre.mermaid');
  for (const pre of nodes) {
    await renderMermaidNode(pre);
  }
}

// stash original source before first render so we can re-render on theme change
for (const pre of document.querySelectorAll('pre.mermaid')) {
  pre.dataset.mermaidSource = pre.innerHTML;
}

mermaid.initialize({ startOnLoad: true, theme: mermaidTheme() });

window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', reinitAllMermaid);

document.addEventListener('hotmeal:opaque-changed', async (e) => {
  if (e.detail?.key !== 'mermaid') return;
  const el = e.detail.element;
  if (!el) return;

  // The opaque patch gives us new HTML content — apply it to the DOM
  if (e.detail.content) {
    el.innerHTML = e.detail.content;
  }

  const pre = el.querySelector('pre.mermaid');
  if (pre) {
    pre.dataset.mermaidSource = pre.textContent;
    await renderMermaidNode(pre);
  }
});
</script>"#;

            Ok(CodeBlockOutput {
                html,
                head_injections: vec![HeadInjection {
                    key: "mermaid".to_string(),
                    html: script.to_string(),
                }],
            })
        })
    }
}

/// A parsed section from a compare block.
#[derive(Debug, Clone)]
pub struct CompareSection {
    /// Language identifier for syntax highlighting
    pub language: String,
    /// The code content
    pub code: String,
}

/// Side-by-side code comparison handler.
///
/// Parses code blocks with `/// language` separators and renders them
/// side-by-side with syntax highlighting.
///
/// # Syntax
///
/// ````text
/// ```compare
/// /// json
/// {"server": {"host": "localhost", "port": 8080}}
/// /// styx
/// server host=localhost port=8080
/// ```
/// ````
///
/// The `/// language` lines act as separators, where `language` is the
/// syntax highlighting language for the following code section.
///
/// # Output
///
/// Renders as a flex container with each section displayed side-by-side.
/// Each section has its language as a header and syntax-highlighted code.
#[cfg(feature = "highlight")]
pub struct CompareHandler {
    highlighter: std::sync::Mutex<arborium::Highlighter>,
}

#[cfg(feature = "highlight")]
impl CompareHandler {
    /// Create a new CompareHandler with default config.
    pub fn new() -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::new()),
        }
    }

    /// Create a new CompareHandler with custom config.
    pub fn with_config(config: arborium::Config) -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::with_config(config)),
        }
    }

    /// Parse the compare block content into sections.
    ///
    /// Each section starts with `/// language` and contains the code until
    /// the next separator or end of content.
    pub fn parse_sections(code: &str) -> Vec<CompareSection> {
        let mut sections = Vec::new();
        let mut current_language: Option<String> = None;
        let mut current_code = String::new();

        for line in code.lines() {
            if let Some(lang) = line.strip_prefix("/// ") {
                // Start a new section - save previous if exists
                if let Some(lang) = current_language.take() {
                    sections.push(CompareSection {
                        language: lang,
                        code: current_code.trim_end().to_string(),
                    });
                    current_code.clear();
                }
                current_language = Some(lang.trim().to_string());
            } else if current_language.is_some() {
                // Accumulate code in current section
                if !current_code.is_empty() {
                    current_code.push('\n');
                }
                current_code.push_str(line);
            }
            // Lines before any `/// language` are ignored
        }

        // Don't forget the last section
        if let Some(lang) = current_language {
            sections.push(CompareSection {
                language: lang,
                code: current_code.trim_end().to_string(),
            });
        }

        sections
    }

    /// Highlight code using arborium, with fallback for unsupported languages.
    fn highlight_code(&self, language: &str, code: &str) -> String {
        use crate::handler::html_escape;

        if language.is_empty() {
            return html_escape(code);
        }

        // Map common language aliases
        let arborium_lang = match language {
            "jinja" => "jinja2",
            _ => language,
        };

        let mut hl = self.highlighter.lock().unwrap();
        match hl.highlight(arborium_lang, code) {
            Ok(html) => html,
            Err(_) => html_escape(code),
        }
    }
}

#[cfg(feature = "highlight")]
impl Default for CompareHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "highlight")]
impl CodeBlockHandler for CompareHandler {
    fn render<'a>(
        &'a self,
        _language: &'a str,
        code: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<CodeBlockOutput>> + Send + 'a>> {
        Box::pin(async move {
            use crate::handler::html_escape;

            let sections = Self::parse_sections(code);

            if sections.is_empty() {
                // No valid sections found - render as plain text
                let escaped = html_escape(code);
                return Ok(format!(
                    "<div class=\"code-block\"><pre><code>{escaped}</code></pre></div>"
                )
                .into());
            }

            let mut html = String::new();
            html.push_str("<div class=\"compare-container\">");

            for section in &sections {
                let highlighted = self.highlight_code(&section.language, &section.code);
                let escaped_lang = html_escape(&section.language);

                html.push_str("<div class=\"compare-section\">");
                html.push_str(&format!(
                    "<div class=\"compare-header\">{}</div>",
                    escaped_lang
                ));
                html.push_str(&format!(
                    "<div class=\"code-block\"><pre><code class=\"language-{}\">{}</code></pre></div>",
                    escaped_lang, highlighted
                ));
                html.push_str("</div>");
            }

            html.push_str("</div>");

            Ok(html.into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "highlight")]
    mod compare_handler_tests {
        use super::*;

        #[test]
        fn test_parse_sections_basic() {
            let code = r#"/// json
{"key": "value"}
/// yaml
key: value"#;

            let sections = CompareHandler::parse_sections(code);
            assert_eq!(sections.len(), 2);

            assert_eq!(sections[0].language, "json");
            assert_eq!(sections[0].code, r#"{"key": "value"}"#);

            assert_eq!(sections[1].language, "yaml");
            assert_eq!(sections[1].code, "key: value");
        }

        #[test]
        fn test_parse_sections_multiline_code() {
            let code = r#"/// rust
fn main() {
    println!("Hello");
}
/// python
def main():
    print("Hello")"#;

            let sections = CompareHandler::parse_sections(code);
            assert_eq!(sections.len(), 2);

            assert_eq!(sections[0].language, "rust");
            assert!(sections[0].code.contains("fn main()"));
            assert!(sections[0].code.contains("println!"));

            assert_eq!(sections[1].language, "python");
            assert!(sections[1].code.contains("def main():"));
        }

        #[test]
        fn test_parse_sections_ignores_leading_content() {
            let code = r#"This is ignored
Also ignored
/// json
{"valid": true}"#;

            let sections = CompareHandler::parse_sections(code);
            assert_eq!(sections.len(), 1);
            assert_eq!(sections[0].language, "json");
            assert_eq!(sections[0].code, r#"{"valid": true}"#);
        }

        #[test]
        fn test_parse_sections_empty() {
            let code = "no sections here";
            let sections = CompareHandler::parse_sections(code);
            assert!(sections.is_empty());
        }

        #[test]
        fn test_parse_sections_three_way() {
            let code = r#"/// json
{"format": "json"}
/// yaml
format: yaml
/// toml
format = "toml""#;

            let sections = CompareHandler::parse_sections(code);
            assert_eq!(sections.len(), 3);
            assert_eq!(sections[0].language, "json");
            assert_eq!(sections[1].language, "yaml");
            assert_eq!(sections[2].language, "toml");
        }

        #[tokio::test]
        async fn test_render_compare_block() {
            let handler = CompareHandler::new();
            let code = r#"/// json
{"key": "value"}
/// yaml
key: value"#;

            let output = handler.render("compare", code).await.unwrap();

            assert!(output.html.contains(r#"class="compare-container""#));
            assert!(output.html.contains(r#"class="compare-section""#));
            assert!(output.html.contains(r#"class="compare-header""#));
            assert!(output.html.contains("json"));
            assert!(output.html.contains("yaml"));
            assert!(output.head_injections.is_empty());
        }

        #[tokio::test]
        async fn test_render_empty_compare_block() {
            let handler = CompareHandler::new();
            let code = "no valid sections";

            let output = handler.render("compare", code).await.unwrap();

            // Should fall back to plain text rendering
            assert!(
                output
                    .html
                    .contains("<div class=\"code-block\"><pre><code>")
            );
            assert!(output.html.contains("no valid sections"));
        }
    }

    mod mermaid_handler_tests {
        use super::*;

        #[tokio::test]
        async fn test_mermaid_handler_output() {
            let handler = MermaidHandler::new();
            let code = "graph TD\n    A-->B";
            let output = handler.render("mermaid", code).await.unwrap();

            // Wrapped in data-hotmeal-opaque
            assert!(
                output.html.contains("data-hotmeal-opaque=\"mermaid\""),
                "Should have hotmeal opaque wrapper: {}",
                output.html
            );
            // Contains pre.mermaid
            assert!(
                output.html.contains("<pre class=\"mermaid\">"),
                "Should have pre.mermaid: {}",
                output.html
            );
            // Code is HTML-escaped
            assert!(
                output.html.contains("A--&gt;B"),
                "Code should be HTML-escaped: {}",
                output.html
            );
            // Head injection present
            assert_eq!(output.head_injections.len(), 1);
            assert_eq!(output.head_injections[0].key, "mermaid");
            assert!(output.head_injections[0].html.contains("mermaid"));
        }
    }
}
