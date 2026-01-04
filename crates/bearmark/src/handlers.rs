//! Built-in code block handlers.
//!
//! These handlers are available when their respective feature flags are enabled:
//! - `highlight` - Syntax highlighting via arborium
//! - `aasvg` - ASCII art to SVG conversion
//! - `pikru` - Pikchr diagram rendering

#[cfg(any(feature = "highlight", feature = "aasvg", feature = "pikru"))]
use std::future::Future;
#[cfg(any(feature = "highlight", feature = "aasvg", feature = "pikru"))]
use std::pin::Pin;

#[cfg(any(feature = "highlight", feature = "aasvg", feature = "pikru"))]
use crate::Result;
#[cfg(any(feature = "highlight", feature = "aasvg", feature = "pikru"))]
use crate::handler::CodeBlockHandler;

/// Syntax highlighting handler using arborium.
///
/// Requires the `highlight` feature.
#[cfg(feature = "highlight")]
pub struct ArboriumHandler {
    highlighter: std::sync::Mutex<arborium::Highlighter>,
}

#[cfg(feature = "highlight")]
impl ArboriumHandler {
    /// Create a new ArboriumHandler with default config.
    pub fn new() -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::new()),
        }
    }

    /// Create a new ArboriumHandler with custom config.
    pub fn with_config(config: arborium::Config) -> Self {
        Self {
            highlighter: std::sync::Mutex::new(arborium::Highlighter::with_config(config)),
        }
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
            // Empty language means no syntax highlighting requested - render as plain
            if language.is_empty() {
                use crate::handler::html_escape;
                let escaped = html_escape(code);
                return Ok(format!("<pre><code>{escaped}</code></pre>"));
            }

            // Map common language aliases to arborium language names
            let arborium_lang = match language {
                "jinja" => "jinja2",
                _ => language,
            };

            // Try to highlight with arborium
            let mut hl = self.highlighter.lock().unwrap();
            match hl.highlight(arborium_lang, code) {
                Ok(html) => {
                    // Wrap arborium's highlighted output in pre/code tags
                    use crate::handler::html_escape;
                    let escaped_lang = html_escape(language);
                    Ok(format!(
                        "<pre><code class=\"language-{escaped_lang}\">{html}</code></pre>"
                    ))
                }
                Err(e) => Err(crate::Error::CodeBlockHandler {
                    language: language.to_string(),
                    message: e.to_string(),
                }),
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
