//! Syntax highlighting plugin for dodeca using arborium/tree-sitter.
//!
//! Provides syntax highlighting for code blocks in markdown with support
//! for 100+ programming languages via tree-sitter grammars.

use facet::Facet;
use plugcard::{plugcard, PlugResult};

plugcard::export_plugin!();

/// Result of syntax highlighting
#[derive(Facet, Debug, Clone)]
pub struct HighlightResult {
    /// The highlighted HTML (with <span> tags for tokens)
    pub html: String,
    /// Whether highlighting was successful (vs fallback to plain text)
    pub highlighted: bool,
}

/// Highlight source code and return HTML with syntax highlighting.
///
/// The returned HTML contains `<span>` elements with classes like:
/// - `hl-keyword` for keywords
/// - `hl-string` for string literals
/// - `hl-comment` for comments
/// - `hl-function` for function names
/// - etc.
///
/// If the language is not recognized or highlighting fails, returns
/// the code as HTML-escaped plain text.
#[plugcard]
pub fn highlight_code(code: String, language: String) -> PlugResult<HighlightResult> {
    // Catch panics from tree-sitter (there are some edge-case bugs)
    match std::panic::catch_unwind(|| highlight_code_inner(&code, &language)) {
        Ok(result) => result,
        Err(_) => {
            // Panic occurred - fallback to escaped plain text
            PlugResult::Ok(HighlightResult {
                html: arborium::html_escape(&code),
                highlighted: false,
            })
        }
    }
}

fn highlight_code_inner(code: &str, language: &str) -> PlugResult<HighlightResult> {
    // Normalize language name (handle common aliases)
    let lang = normalize_language(language);

    // Create highlighter with static provider
    let provider = arborium::StaticProvider::default();
    let mut highlighter = arborium::SyncHighlighter::new(provider);

    // Attempt to highlight
    match highlighter.highlight(&lang, code) {
        Ok(html) => PlugResult::Ok(HighlightResult {
            html,
            highlighted: true,
        }),
        Err(_) => {
            // Fallback to escaped plain text
            PlugResult::Ok(HighlightResult {
                html: arborium::html_escape(code),
                highlighted: false,
            })
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

/// Get list of supported languages.
#[plugcard]
pub fn supported_languages() -> PlugResult<Vec<String>> {
    // Return a static list of commonly used languages
    // arborium supports 100+ languages via feature flags
    PlugResult::Ok(vec![
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
    ])
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_rust() {
        let code = "fn main() {}";
        let result = highlight_code(code.to_string(), "rust".to_string());
        let PlugResult::Ok(result) = result else {
            panic!("Expected Ok");
        };
        assert!(result.highlighted);
        // arborium uses custom elements like <a-k> for keywords
        assert!(result.html.contains("<a-"));
    }

    #[test]
    fn test_highlight_unknown_language() {
        let code = "some code";
        let result = highlight_code(code.to_string(), "nonexistent-lang-xyz".to_string());
        let PlugResult::Ok(result) = result else {
            panic!("Expected Ok");
        };
        // Should still succeed but with plain text fallback
        assert!(!result.highlighted || result.html.contains("some code"));
    }

    #[test]
    fn test_language_normalization() {
        assert_eq!(normalize_language("js"), "javascript");
        assert_eq!(normalize_language("ts"), "typescript");
        assert_eq!(normalize_language("py"), "python");
        assert_eq!(normalize_language("rs"), "rust");
        assert_eq!(normalize_language("JS"), "javascript"); // case insensitive
    }

    #[test]
    fn test_various_languages() {
        let test_cases = vec![
            ("bash", "mkdir my-site"),
            ("bash", "cd my-site"),
            ("bash", "mkdir my-site\ncd my-site"),
            ("kdl", "content \"content\"\noutput \"public\""),
            ("markdown", "+++\ntitle = \"My Site\"\n+++\n\nHello, world!"),
            ("html", "<!DOCTYPE html>"),
            ("rust", "let message = \"Hello from dodeca!\";"),
        ];

        for (lang, code) in test_cases {
            println!("Testing {}: {:?} (len={})", lang, code, code.len());
            // Fresh highlighter each time to test isolation
            let provider = arborium::StaticProvider::default();
            let mut highlighter = arborium::SyncHighlighter::new(provider);
            let result = highlighter.highlight(lang, code);
            println!("  Result: {:?}", result.is_ok());
        }
    }

    #[test]
    fn test_reused_highlighter_bug() {
        // This test reproduces a bug where reusing a SyncHighlighter
        // causes slice bounds errors when the second string is shorter
        let provider = arborium::StaticProvider::default();
        let mut highlighter = arborium::SyncHighlighter::new(provider);

        // First call with longer string
        let result1 = highlighter.highlight("bash", "mkdir my-site"); // 13 bytes
        println!("First: {:?}", result1.is_ok());

        // Second call with shorter string - this panics!
        let result2 = highlighter.highlight("bash", "cd my-site"); // 10 bytes
        println!("Second: {:?}", result2.is_ok());
    }
}
