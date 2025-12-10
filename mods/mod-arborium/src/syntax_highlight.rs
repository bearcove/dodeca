//! Syntax highlighting implementation for the rapace plugin

use std::sync::Arc;

use dodeca_syntax_highlight_protocol::{HighlightResult, SyntaxHighlightService};

/// Syntax highlighting implementation
pub struct SyntaxHighlightImpl;

impl SyntaxHighlightService for SyntaxHighlightImpl {
    async fn highlight_code(&self, code: String, language: String) -> HighlightResult {
        // Catch panics from tree-sitter (there are some edge-case bugs)
        match std::panic::catch_unwind(|| highlight_code_inner(&code, &language)) {
            Ok(result) => result,
            Err(_) => {
                // Panic occurred - fallback to escaped plain text
                HighlightResult {
                    html: arborium::html_escape(&code),
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

    // Create highlighter with static provider
    let provider = arborium::StaticProvider::default();
    let mut highlighter = arborium::SyncHighlighter::new(provider);

    // Attempt to highlight
    match highlighter.highlight(&lang, code) {
        Ok(html) => HighlightResult {
            html,
            highlighted: true,
        },
        Err(_) => {
            // Fallback to escaped plain text
            HighlightResult {
                html: arborium::html_escape(code),
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
