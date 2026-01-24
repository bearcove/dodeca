//! Error page rendering utilities
//!
//! Provides two formatting paths for template errors:
//! - ANSI output for CLI (build mode)
//! - HTML output for web (dev server mode)

use ariadne::{Color, Label, Report, ReportKind, Source};
use cell_gingembre_proto::TemplateRenderError;

pub use dodeca_protocol::ansi_to_html;

/// Marker for render errors (data attribute survives HTML minification)
pub const RENDER_ERROR_MARKER: &str = "data-dodeca-error";

/// Format a structured template error to ANSI for CLI output.
/// Uses ariadne for pretty source context display.
pub fn format_error_ansi(error: &TemplateRenderError) -> String {
    if let Some(ref loc) = error.location {
        // Build ariadne report with source context
        let start = loc.offset;
        let end = start + loc.length.max(1);

        let mut report = Report::build(ReportKind::Error, (&loc.filename, start..end))
            .with_message(&error.message);

        // Add the primary label
        let label = Label::new((&loc.filename, start..end))
            .with_message(&error.message)
            .with_color(Color::Red);
        report = report.with_label(label);

        // Add help if available
        if let Some(ref help) = error.help {
            report = report.with_help(help);
        }

        // Render to string
        let mut output = Vec::new();
        report
            .finish()
            .write((&loc.filename, Source::from(&loc.source)), &mut output)
            .expect("failed to write error report");

        String::from_utf8(output).expect("ariadne produced invalid UTF-8")
    } else {
        // No source location, just return the message
        if let Some(ref help) = error.help {
            format!("Error: {}\n\nHelp: {}", error.message, help)
        } else {
            format!("Error: {}", error.message)
        }
    }
}

/// Dodeca logo SVG for error pages
const DODECA_LOGO_SVG: &str = include_str!("../logo.svg");

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

/// Render a generic error page (for parse errors, etc.)
pub fn render_generic_error_page(title: &str, message: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en" {RENDER_ERROR_MARKER}>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title} - dodeca</title>
    <style>{ERROR_PAGE_STYLES}</style>
</head>
<body>
    <div class="dev-badge">dev</div>
    <div class="container">
        <div class="logo">{DODECA_LOGO_SVG}</div>
        <h1>{title}</h1>
        <pre>{message}</pre>
    </div>
</body>
</html>"#,
        title = html_escape::encode_text(title),
        message = html_escape::encode_text(message),
    )
}

/// Render a structured template error to an HTML error page
pub fn render_structured_error_page(error: &TemplateRenderError) -> String {
    let error_html = format_error_html(error);
    format!(
        r#"<!DOCTYPE html>
<html lang="en" {RENDER_ERROR_MARKER}>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Template Error - dodeca</title>
    <style>{ERROR_PAGE_STYLES}{ERROR_SOURCE_STYLES}</style>
</head>
<body>
    <div class="dev-badge">dev</div>
    <div class="container error-container">
        <div class="logo">{DODECA_LOGO_SVG}</div>
        <h1>Template Render Error</h1>
        {error_html}
    </div>
</body>
</html>"#
    )
}

/// Additional CSS for source code display
const ERROR_SOURCE_STYLES: &str = r#"
.container.error-container {
    max-width: 1200px;
}
.error-message {
    color: #f87171;
    font-weight: 500;
    margin: 0.5rem 0;
    font-size: 0.95rem;
}
.source-context {
    background: #0d0d0d;
    border: 1px solid #333;
    border-radius: 8px;
    padding: 1rem;
    overflow-x: auto;
    font-family: 'SF Mono', Consolas, 'Liberation Mono', monospace;
    font-size: 13px;
    line-height: 1.6;
}
.source-line {
    display: flex;
    white-space: pre;
}
.line-number {
    color: #525252;
    min-width: 3ch;
    text-align: right;
    padding-right: 1rem;
    user-select: none;
}
.line-content {
    color: #ccc;
}
.error-line .line-content {
    background: rgba(248, 113, 113, 0.1);
}
.error-indicator-row {
    display: flex;
    padding-left: calc(3ch + 1rem);
    white-space: pre;
    font-family: 'SF Mono', Consolas, 'Liberation Mono', monospace;
}
.error-marker {
    color: #f87171;
    font-weight: bold;
}
.error-location-link {
    display: inline-block;
    background: #262626;
    padding: 0.25rem 0.75rem;
    border-radius: 4px;
    font-family: 'SF Mono', Consolas, monospace;
    font-size: 0.85rem;
    color: #6a8a6a;
    text-decoration: none;
    border: 1px solid #333;
    margin-bottom: 0.5rem;
}
.error-location-link:hover {
    background: #333;
    color: #8fbc8f;
}
"#;

/// Format a structured error to HTML with source context
fn format_error_html(error: &TemplateRenderError) -> String {
    let mut html = String::new();

    // Source context if available (error message shown inline)
    if let Some(ref loc) = error.location {
        html.push_str(&format_source_context(
            loc,
            &error.message,
            error.help.as_deref(),
        ));
    } else {
        // No location - just show message
        html.push_str(&format!(
            r#"<div class="error-message">{}</div>"#,
            html_escape::encode_text(&error.message)
        ));
        if let Some(ref help) = error.help {
            html.push_str(&format!(
                r#"<div class="hint"><strong>Hint:</strong> {}</div>"#,
                html_escape::encode_text(help)
            ));
        }
    }

    html
}

/// Format source context with line numbers and error highlighting
fn format_source_context(
    loc: &cell_gingembre_proto::ErrorLocation,
    message: &str,
    help: Option<&str>,
) -> String {
    // Assert that we have an absolute path for proper editor linking
    assert!(
        loc.filename.starts_with('/'),
        "Template error location must be an absolute path, got: {:?}",
        loc.filename
    );

    let mut html = String::new();

    // Calculate line/column from offset
    let (error_line, error_col) = offset_to_line_col(&loc.source, loc.offset);

    html.push_str(r#"<div class="source-context">"#);

    // Get lines around the error
    let lines: Vec<&str> = loc.source.lines().collect();
    let start_line = error_line.saturating_sub(3).max(1);
    let end_line = (error_line + 2).min(lines.len());

    for (i, line) in lines.iter().enumerate().take(end_line).skip(start_line - 1) {
        let line_num = i + 1;
        let is_error_line = line_num == error_line;
        let class = if is_error_line {
            "source-line error-line"
        } else {
            "source-line"
        };

        html.push_str(&format!(
            r#"<div class="{}"><span class="line-number">{}</span><span class="line-content">{}</span></div>"#,
            class,
            line_num,
            html_escape::encode_text(line)
        ));

        // Add error indicator and message under the error line
        if is_error_line {
            let indicator_offset = error_col.saturating_sub(1);
            let indicator_len = loc.length.max(1);
            let spaces = " ".repeat(indicator_offset);
            let markers = "^".repeat(indicator_len);

            html.push_str(&format!(
                r#"<div class="error-indicator-row"><span class="error-marker">{}{} {}</span></div>"#,
                spaces,
                markers,
                html_escape::encode_text(message)
            ));
        }
    }

    html.push_str("</div>");

    // Show filename/location - clickable if it's an absolute path
    let location_text = format!(
        "{}:{}:{}",
        html_escape::encode_text(&loc.filename),
        error_line,
        error_col
    );
    if loc.filename.starts_with('/') {
        // Absolute path - make it a zed:// link
        // Note: zed://file/ expects path without leading slash
        let zed_url = format!("zed://file{}:{}:{}", &loc.filename, error_line, error_col);
        html.push_str(&format!(
            r#"<a class="error-location-link" href="{}">{}</a>"#,
            html_escape::encode_text(&zed_url),
            location_text
        ));
    } else {
        // Relative path - just show as text (styled like a link but not clickable)
        html.push_str(&format!(
            r#"<span class="error-location-link">{}</span>"#,
            location_text
        ));
    }

    // Add help if present
    if let Some(help_text) = help {
        html.push_str(&format!(
            r#"<div class="hint"><strong>Hint:</strong> {}</div>"#,
            html_escape::encode_text(help_text)
        ));
    }

    html
}

/// Calculate line and column from byte offset (1-indexed)
pub fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_structured_error_page_contains_marker() {
        let error = TemplateRenderError {
            message: "test error".to_string(),
            location: None,
            help: None,
        };
        let html = render_structured_error_page(&error);
        assert!(html.contains(RENDER_ERROR_MARKER));
        assert!(html.contains("test error"));
    }

    #[test]
    fn test_render_generic_error_page_contains_marker() {
        let html = render_generic_error_page("Parse Error", "test error");
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
