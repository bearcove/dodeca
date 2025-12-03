//! Error page rendering utilities
//!
//! Provides consistent error page styling for:
//! - Template render errors (development mode)
//! - 404 pages (development mode)
//!
//! Includes ANSI-to-HTML conversion for displaying colorized error messages.

/// Marker for render errors (used to detect errors in production builds)
pub const RENDER_ERROR_MARKER: &str = "<!-- DODECA_RENDER_ERROR -->";

/// Dodeca logo SVG for error pages
const DODECA_LOGO_SVG: &str = include_str!("../docs/static/logo.svg");

/// Shared CSS styles for all error pages
const ERROR_PAGE_STYLES: &str = r#"
body {
    font-family: system-ui, -apple-system, sans-serif;
    background: #1a1a1a;
    color: #d4d4d4;
    margin: 0;
    padding: 0;
    min-height: 100vh;
}
.container {
    max-width: 700px;
    margin: 0 auto;
    padding: 2rem;
}
.logo {
    text-align: center;
    margin-bottom: 1.5rem;
    animation: float 3s ease-in-out infinite;
}
.logo svg {
    width: 60px;
    height: 60px;
    opacity: 0.5;
}
@keyframes float {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-6px); }
}
h1 {
    color: #e5e5e5;
    font-size: 1.5rem;
    margin-bottom: 0.5rem;
    font-weight: 500;
    text-align: center;
}
.subtitle {
    color: #737373;
    text-align: center;
    margin-bottom: 1.5rem;
}
pre {
    background: #0d0d0d;
    border: 1px solid #333;
    border-radius: 8px;
    padding: 1rem;
    overflow-x: auto;
    white-space: pre-wrap;
    word-wrap: break-word;
    font-size: 13px;
    line-height: 1.6;
    color: #ccc;
    font-family: 'SF Mono', Consolas, 'Liberation Mono', monospace;
}
.path {
    background: #262626;
    padding: 0.5rem 1rem;
    border-radius: 6px;
    font-family: 'SF Mono', Consolas, monospace;
    font-size: 0.9rem;
    color: #a3a3a3;
    margin: 1rem auto;
    max-width: fit-content;
    word-break: break-all;
    border: 1px solid #333;
}
.hint {
    background: #252525;
    border-left: 3px solid #525252;
    padding: 1rem;
    margin-top: 1.5rem;
    color: #a3a3a3;
    font-size: 0.9rem;
}
.hint strong {
    color: #d4d4d4;
}
.suggestions {
    margin-top: 2rem;
}
.suggestions h2 {
    font-size: 0.8rem;
    color: #737373;
    margin-bottom: 0.75rem;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}
.suggestions ul {
    list-style: none;
    padding: 0;
    margin: 0;
}
.suggestions li {
    padding: 0.5rem 0;
    border-bottom: 1px solid #262626;
}
.suggestions li:last-child {
    border-bottom: none;
}
.suggestions a {
    color: #6a8a6a;
    text-decoration: none;
    transition: color 0.2s;
}
.suggestions a:hover {
    color: #8fbc8f;
    text-decoration: underline;
}
.no-results {
    color: #525252;
    font-style: italic;
}
.actions {
    margin-top: 2rem;
    display: flex;
    gap: 1rem;
    justify-content: center;
}
.btn {
    padding: 0.5rem 1rem;
    border-radius: 6px;
    text-decoration: none;
    font-weight: 500;
    font-size: 0.875rem;
    transition: all 0.2s;
}
.btn:hover {
    transform: translateY(-1px);
}
.btn-primary {
    background: #6a8a6a;
    color: #fff;
}
.btn-primary:hover {
    background: #7a9a7a;
}
.btn-secondary {
    background: #262626;
    color: #a3a3a3;
    border: 1px solid #333;
}
.btn-secondary:hover {
    background: #333;
    color: #d4d4d4;
}
.dev-badge {
    position: fixed;
    top: 1rem;
    right: 1rem;
    background: #333;
    color: #737373;
    padding: 0.25rem 0.75rem;
    border-radius: 4px;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}
"#;

/// Render a template error page for development mode
pub fn render_error_page(error: &str) -> String {
    let error_html = ansi_to_html(error);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
{RENDER_ERROR_MARKER}
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Template Error - dodeca</title>
    <style>{ERROR_PAGE_STYLES}</style>
</head>
<body>
    <div class="dev-badge">dev</div>
    <div class="container">
        <div class="logo">{DODECA_LOGO_SVG}</div>
        <h1>Template Render Error</h1>
        <pre>{error_html}</pre>
        <div class="hint">
            <strong>Hint:</strong> Check your template syntax and ensure all referenced variables exist.
        </div>
    </div>
</body>
</html>"#
    )
}

/// Render a helpful 404 page for development mode
pub fn render_404_page(path: &str, similar_routes: &[(String, String)]) -> String {
    let suggestions = if similar_routes.is_empty() {
        "<p class=\"no-results\">No similar pages found.</p>".to_string()
    } else {
        let links: Vec<String> = similar_routes
            .iter()
            .map(|(route, title)| {
                let display_title = if title.is_empty() {
                    route.clone()
                } else {
                    format!("{} ({})", title, route)
                };
                format!(r#"<li><a href="{}">{}</a></li>"#, route, display_title)
            })
            .collect();
        format!("<ul>{}</ul>", links.join("\n"))
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Page Not Found - dodeca</title>
    <style>{ERROR_PAGE_STYLES}</style>
</head>
<body>
    <div class="dev-badge">dev</div>
    <div class="container" style="text-align: center;">
        <div class="logo">{DODECA_LOGO_SVG}</div>
        <h1>Page Not Found</h1>
        <p class="subtitle">The page you're looking for doesn't exist (yet?).</p>
        <div class="path">{path}</div>
        <div class="suggestions" style="text-align: left;">
            <h2>Maybe you meant</h2>
            {suggestions}
        </div>
        <div class="actions">
            <a href="javascript:history.back()" class="btn btn-secondary">← Go Back</a>
            <a href="/" class="btn btn-primary">Home</a>
        </div>
    </div>
</body>
</html>"#
    )
}

// ============================================================================
// ANSI to HTML conversion
// ============================================================================

/// Convert ANSI escape codes to HTML spans with inline styles.
///
/// Supports:
/// - Basic styles: bold, dim, italic, underline
/// - Standard colors (30-37, 90-97)
/// - 24-bit RGB colors (38;2;r;g;b)
pub fn ansi_to_html(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    let mut in_span = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['

            // Parse the escape sequence
            let mut seq = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_ascii_digit() || ch == ';' {
                    seq.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Consume the final character (usually 'm')
            let final_char = chars.next();

            if final_char == Some('m') {
                // Close any existing span
                if in_span {
                    output.push_str("</span>");
                    in_span = false;
                }

                // Parse the style
                if let Some(style) = parse_ansi_style(&seq) {
                    if !style.is_empty() {
                        output.push_str(&format!("<span style=\"{style}\">"));
                        in_span = true;
                    }
                }
            }
        } else if c == '<' {
            output.push_str("&lt;");
        } else if c == '>' {
            output.push_str("&gt;");
        } else if c == '&' {
            output.push_str("&amp;");
        } else {
            output.push(c);
        }
    }

    if in_span {
        output.push_str("</span>");
    }

    output
}

/// Parse ANSI style codes and return CSS style string.
fn parse_ansi_style(seq: &str) -> Option<String> {
    if seq.is_empty() || seq == "0" {
        return Some(String::new()); // Reset
    }

    let parts: Vec<&str> = seq.split(';').collect();
    let mut styles = Vec::new();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "0" => return Some(String::new()), // Reset
            "1" => styles.push("font-weight:bold".to_string()),
            "2" => styles.push("opacity:0.7".to_string()), // Dim
            "3" => styles.push("font-style:italic".to_string()),
            "4" => styles.push("text-decoration:underline".to_string()),
            "30" => styles.push("color:#000".to_string()),
            "31" => styles.push("color:#e06c75".to_string()), // Red
            "32" => styles.push("color:#98c379".to_string()), // Green
            "33" => styles.push("color:#e5c07b".to_string()), // Yellow
            "34" => styles.push("color:#61afef".to_string()), // Blue
            "35" => styles.push("color:#c678dd".to_string()), // Magenta
            "36" => styles.push("color:#56b6c2".to_string()), // Cyan
            "37" => styles.push("color:#abb2bf".to_string()), // White
            "38" => {
                // Extended color (24-bit RGB)
                if i + 1 < parts.len() && parts[i + 1] == "2" && i + 4 < parts.len() {
                    let r = parts[i + 2];
                    let g = parts[i + 3];
                    let b = parts[i + 4];
                    styles.push(format!("color:rgb({r},{g},{b})"));
                    i += 4;
                }
            }
            "90" => styles.push("color:#5c6370".to_string()), // Bright black (gray)
            "91" => styles.push("color:#e06c75".to_string()), // Bright red
            "92" => styles.push("color:#98c379".to_string()), // Bright green
            "93" => styles.push("color:#e5c07b".to_string()), // Bright yellow
            "94" => styles.push("color:#61afef".to_string()), // Bright blue
            "95" => styles.push("color:#c678dd".to_string()), // Bright magenta
            "96" => styles.push("color:#56b6c2".to_string()), // Bright cyan
            "97" => styles.push("color:#fff".to_string()),    // Bright white
            _ => {}
        }
        i += 1;
    }

    Some(styles.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ansi_to_html_plain_text() {
        assert_eq!(ansi_to_html("hello world"), "hello world");
    }

    #[test]
    fn test_ansi_to_html_escapes_html() {
        assert_eq!(ansi_to_html("<script>"), "&lt;script&gt;");
        assert_eq!(ansi_to_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_ansi_to_html_bold() {
        let input = "\x1b[1mbold\x1b[0m normal";
        let output = ansi_to_html(input);
        assert!(output.contains("font-weight:bold"));
        assert!(output.contains("bold</span>"));
    }

    #[test]
    fn test_ansi_to_html_colors() {
        let input = "\x1b[31mred\x1b[0m";
        let output = ansi_to_html(input);
        assert!(output.contains("color:#e06c75"));
    }

    #[test]
    fn test_render_error_page_contains_marker() {
        let html = render_error_page("test error");
        assert!(html.contains(RENDER_ERROR_MARKER));
        assert!(html.contains("test error"));
    }

    #[test]
    fn test_render_404_page() {
        let html = render_404_page("/missing", &[]);
        assert!(html.contains("/missing"));
        assert!(html.contains("Page Not Found"));
    }

    #[test]
    fn test_render_404_page_with_suggestions() {
        let suggestions = vec![
            ("/about".to_string(), "About Us".to_string()),
            ("/contact".to_string(), "".to_string()),
        ];
        let html = render_404_page("/abut", &suggestions);
        assert!(html.contains("/about"));
        assert!(html.contains("About Us"));
        assert!(html.contains("/contact"));
    }
}
