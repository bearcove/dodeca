//! Built-in code block handlers.
//!
//! These handlers are available when their respective feature flags are enabled:
//! - `highlight` - Syntax highlighting via arborium
//! - `aasvg` - ASCII art to SVG conversion
//! - `pikru` - Pikchr diagram rendering

use std::future::Future;
use std::pin::Pin;

use crate::Result;
use crate::handler::CodeBlockHandler;

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
            show_language_header: false,
        }
    }

    /// Create a new ArboriumHandler with custom config.
    pub fn with_config(config: arborium::Config) -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::with_config(config)),
            show_language_header: false,
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
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            use crate::handler::html_escape;

            // Empty language means no syntax highlighting requested - render as plain
            if language.is_empty() {
                let escaped = html_escape(code);
                return Ok(format!(
                    "<div class=\"code-block\"><pre><code>{escaped}</code></pre></div>"
                ));
            }

            // Map common language aliases to arborium language names
            let arborium_lang = match language {
                "jinja" => "jinja2",
                _ => language,
            };

            let escaped_lang = html_escape(language);

            // Try to highlight with arborium
            let mut hl = self.highlighter.lock().unwrap();
            let code_html = match hl.highlight(arborium_lang, code) {
                Ok(html) => {
                    format!(
                        "<div class=\"code-block\"><pre><code class=\"language-{escaped_lang}\">{html}</code></pre></div>"
                    )
                }
                Err(_e) => {
                    // Fall back to plain text rendering for unsupported languages
                    let escaped = html_escape(code);
                    format!(
                        "<div class=\"code-block\"><pre><code class=\"language-{escaped_lang}\">{escaped}</code></pre></div>"
                    )
                }
            };

            // Wrap with header if enabled
            if self.show_language_header {
                Ok(format!(
                    "<div class=\"code-block\"><div class=\"code-header\">{escaped_lang}</div>{code_html}</div>"
                ))
            } else {
                Ok(code_html)
            }
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
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let svg = aasvg::render(code);
            Ok(svg)
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
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
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
                Ok(svg) => Ok(svg),
                Err(e) => Err(crate::Error::CodeBlockHandler {
                    language: "pik".to_string(),
                    message: format!("render error: {}", e),
                }),
            }
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
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            use crate::handler::html_escape;

            let sections = Self::parse_sections(code);

            if sections.is_empty() {
                // No valid sections found - render as plain text
                let escaped = html_escape(code);
                return Ok(format!(
                    "<div class=\"code-block\"><pre><code>{escaped}</code></pre></div>"
                ));
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

            Ok(html)
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

            let result = handler.render("compare", code).await.unwrap();

            assert!(result.contains(r#"class="compare-container""#));
            assert!(result.contains(r#"class="compare-section""#));
            assert!(result.contains(r#"class="compare-header""#));
            assert!(result.contains("json"));
            assert!(result.contains("yaml"));
        }

        #[tokio::test]
        async fn test_render_empty_compare_block() {
            let handler = CompareHandler::new();
            let code = "no valid sections";

            let result = handler.render("compare", code).await.unwrap();

            // Should fall back to plain text rendering
            assert!(result.contains("<div class=\"code-block\"><pre><code>"));
            assert!(result.contains("no valid sections"));
        }
    }
}
