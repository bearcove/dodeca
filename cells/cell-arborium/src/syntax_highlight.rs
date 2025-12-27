//! Syntax highlighting implementation for the rapace cell

use arborium::advanced::html_escape;
use cell_arborium_proto::{HighlightResult, SyntaxHighlightService};

/// Syntax highlighting implementation
#[derive(Clone)]
pub struct SyntaxHighlightImpl;

impl SyntaxHighlightService for SyntaxHighlightImpl {
    async fn highlight_code(&self, code: String, language: String) -> HighlightResult {
        // For Rust code, filter out hidden lines (doctest-style # prefix)
        // before highlighting for display
        let display_code = if is_rust_like(&language) {
            filter_hidden_lines(&code)
        } else {
            code.clone()
        };

        // Catch panics from tree-sitter (there are some edge-case bugs)
        match std::panic::catch_unwind(|| highlight_code_inner(&display_code, &language)) {
            Ok(result) => result,
            Err(_) => {
                // Panic occurred - fallback to escaped plain text
                HighlightResult {
                    html: html_escape(&display_code),
                    highlighted: false,
                }
            }
        }
    }

    async fn supported_languages(&self) -> Vec<String> {
        // Return a static list of commonly used languages
        // arborium supports 100+ languages via feature flags
        vec![
            "bash".to_string(),
            "c".to_string(),
            "cpp".to_string(),
            "css".to_string(),
            "dockerfile".to_string(),
            "go".to_string(),
            "haskell".to_string(),
            "html".to_string(),
            "java".to_string(),
            "javascript".to_string(),
            "json".to_string(),
            "kotlin".to_string(),
            "lua".to_string(),
            "markdown".to_string(),
            "python".to_string(),
            "ruby".to_string(),
            "rust".to_string(),
            "scala".to_string(),
            "sql".to_string(),
            "swift".to_string(),
            "toml".to_string(),
            "typescript".to_string(),
            "yaml".to_string(),
            "zig".to_string(),
        ]
    }
}

/// Highlight source code and return HTML with syntax highlighting.
fn highlight_code_inner(code: &str, language: &str) -> HighlightResult {
    // Normalize language name (handle common aliases)
    let lang = normalize_language(language);

    // Create highlighter
    let mut highlighter = arborium::Highlighter::new();

    // Attempt to highlight
    match highlighter.highlight(&lang, code) {
        Ok(html) => HighlightResult {
            html,
            highlighted: true,
        },
        Err(_) => {
            // Fallback to escaped plain text
            HighlightResult {
                html: html_escape(code),
                highlighted: false,
            }
        }
    }
}

/// Normalize common language aliases to arborium-recognized names.
fn normalize_language(lang: &str) -> String {
    // Strip anything after comma (e.g., "rust,noexec" -> "rust")
    let lang = lang.split(',').next().unwrap_or(lang);
    let lang = lang.to_lowercase();
    match lang.as_str() {
        // Common aliases
        "js" => "javascript".to_string(),
        "ts" => "typescript".to_string(),
        "py" => "python".to_string(),
        "rb" => "ruby".to_string(),
        "rs" => "rust".to_string(),
        "sh" | "bash" | "zsh" => "bash".to_string(),
        "yml" => "yaml".to_string(),
        "md" => "markdown".to_string(),
        "dockerfile" => "dockerfile".to_string(),
        "c++" | "cc" | "cxx" => "cpp".to_string(),
        "c#" | "csharp" => "c_sharp".to_string(),
        "f#" | "fsharp" => "f_sharp".to_string(),
        "obj-c" | "objc" | "objective-c" => "objc".to_string(),
        "shell" => "bash".to_string(),
        "console" => "bash".to_string(),
        "plaintext" | "text" | "plain" | "" => "text".to_string(),
        _ => lang,
    }
}

/// Check if a language is Rust-like (supports doctest-style hidden lines)
fn is_rust_like(lang: &str) -> bool {
    let lang = lang.split(',').next().unwrap_or(lang).to_lowercase();
    matches!(lang.as_str(), "rust" | "rs")
}

/// Filter out hidden lines from code for display purposes.
///
/// This follows Rust doctest conventions:
/// - `# ` (hash + space at start of line) - hidden from display
/// - `##` at start of line - displays as `#` (escape sequence)
/// - `#[attr]` - NOT hidden, normal Rust attribute syntax
fn filter_hidden_lines(code: &str) -> String {
    code.lines()
        .filter_map(|line| {
            if line.starts_with("# ") {
                // `# code` - hidden line, exclude from display
                None
            } else if let Some(rest) = line.strip_prefix("##") {
                // `##foo` -> `#foo` (escape sequence)
                Some(format!("#{}", rest))
            } else {
                // Normal line (including `#[attr]`), keep as-is
                Some(line.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
