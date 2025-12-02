//! Static HTML/CSS analysis for font usage detection
//!
//! Parses HTML and CSS to determine which characters are used with which fonts,
//! without requiring a browser.

use scraper::{Html, Selector};
use std::collections::{HashMap, HashSet};

/// CSS custom properties (variables) map
type CssVariables = HashMap<String, String>;

/// A parsed @font-face rule
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // weight and style reserved for font-weight matching
pub struct FontFace {
    /// The font-family name declared in @font-face
    pub family: String,
    /// The URL to the font file (from src)
    pub src: String,
    /// Font weight (e.g., "400", "bold")
    pub weight: Option<String>,
    /// Font style (e.g., "normal", "italic")
    pub style: Option<String>,
}

/// Result of analyzing CSS for font information
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontAnalysis {
    /// Map of font-family name -> characters used
    pub chars_per_font: HashMap<String, HashSet<char>>,
    /// Parsed @font-face rules
    pub font_faces: Vec<FontFace>,
}

/// Analyze HTML and CSS to collect font usage information
pub fn analyze_fonts(html: &str, css: &str) -> FontAnalysis {
    let chars_per_font = collect_chars_per_font(html, css);
    let font_faces = parse_font_face_rules(css);

    FontAnalysis {
        chars_per_font,
        font_faces,
    }
}

/// Extracts all text content and maps it to font-families based on CSS rules.
///
/// Returns a map of font-family name -> set of characters used with that font.
pub fn collect_chars_per_font(html: &str, css: &str) -> HashMap<String, HashSet<char>> {
    let document = Html::parse_document(html);

    // First, parse CSS custom properties (variables)
    let css_vars = parse_css_custom_properties(css);

    // Parse font-family rules with variable resolution
    let font_rules = parse_font_family_rules_with_vars(css, &css_vars);

    let mut result: HashMap<String, HashSet<char>> = HashMap::new();

    // For each element with text, determine which font-family applies
    // by checking CSS rules in order of specificity (simplified: last match wins)
    let all_elements = Selector::parse("*").unwrap();

    for element in document.select(&all_elements) {
        // Get direct text content (not from children)
        let text: String = element
            .text()
            .next()
            .map(|s| s.to_string())
            .unwrap_or_default();

        if text.trim().is_empty() {
            continue;
        }

        // Find which font-family applies to this element
        let font_family = find_font_family_for_element(&element, &font_rules)
            .unwrap_or_else(|| "sans-serif".to_string());

        // Add characters to that font's set
        let chars = result.entry(font_family).or_default();
        for c in text.chars() {
            chars.insert(c);
        }
    }

    result
}

/// A CSS rule that sets font-family
#[derive(Debug)]
struct FontFamilyRule {
    selector: String,
    font_family: String,
}

/// Parse CSS and extract rules that set font-family, with CSS variable resolution
fn parse_font_family_rules_with_vars(css: &str, css_vars: &CssVariables) -> Vec<FontFamilyRule> {
    let mut rules = Vec::new();

    // Simple CSS parser - find rule blocks and extract font-family
    // This is a simplified parser that handles basic cases
    let chars = css.chars().peekable();
    let mut current_selector = String::new();
    let mut in_block = false;
    let mut block_content = String::new();

    for c in chars {
        if c == '{' {
            in_block = true;
            block_content.clear();
        } else if c == '}' {
            in_block = false;

            // Parse the block content for font-family
            if let Some(font_family) = extract_font_family_with_vars(&block_content, css_vars) {
                let selector = current_selector.trim().to_string();
                if !selector.is_empty() && !selector.starts_with('@') {
                    rules.push(FontFamilyRule {
                        selector,
                        font_family,
                    });
                }
            }

            current_selector.clear();
        } else if in_block {
            block_content.push(c);
        } else {
            current_selector.push(c);
        }
    }

    rules
}

/// Extract font-family value from a CSS declaration block, with CSS variable resolution
fn extract_font_family_with_vars(block: &str, css_vars: &CssVariables) -> Option<String> {
    // Look for font-family: value; or font: ... value;
    for declaration in block.split(';') {
        let declaration = declaration.trim();

        if let Some(value) = declaration.strip_prefix("font-family:") {
            return Some(parse_font_family_value_with_vars(value, css_vars));
        }

        // Handle shorthand 'font' property (simplified - just look for font-family at end)
        if declaration.starts_with("font:") {
            // The font shorthand is complex; for now just skip it
            // TODO: properly parse font shorthand
        }
    }

    None
}

/// Parse @font-face rules from CSS
fn parse_font_face_rules(css: &str) -> Vec<FontFace> {
    let mut faces = Vec::new();

    // Find all @font-face blocks
    let mut remaining = css;
    while let Some(start) = remaining.find("@font-face") {
        remaining = &remaining[start + "@font-face".len()..];

        // Find the opening brace
        let Some(brace_start) = remaining.find('{') else {
            break;
        };
        remaining = &remaining[brace_start + 1..];

        // Find matching closing brace (handle nested braces)
        let mut depth = 1;
        let mut block_end = 0;
        for (i, c) in remaining.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        block_end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if block_end == 0 {
            break;
        }

        let block = &remaining[..block_end];
        remaining = &remaining[block_end + 1..];

        // Parse the @font-face block
        if let Some(face) = parse_font_face_block(block) {
            faces.push(face);
        }
    }

    faces
}

/// Parse a single @font-face block content
fn parse_font_face_block(block: &str) -> Option<FontFace> {
    let mut family = None;
    let mut src = None;
    let mut weight = None;
    let mut style = None;

    for declaration in block.split(';') {
        let declaration = declaration.trim();

        if let Some(value) = declaration.strip_prefix("font-family:") {
            family = Some(parse_font_family_value(value));
        } else if let Some(value) = declaration.strip_prefix("src:") {
            src = parse_font_src(value);
        } else if let Some(value) = declaration.strip_prefix("font-weight:") {
            weight = Some(value.trim().to_string());
        } else if let Some(value) = declaration.strip_prefix("font-style:") {
            style = Some(value.trim().to_string());
        }
    }

    Some(FontFace {
        family: family?,
        src: src?,
        weight,
        style,
    })
}

/// Parse the src property of @font-face
/// Handles: url("/path/to/font.woff2"), url('/path'), url(path)
fn parse_font_src(value: &str) -> Option<String> {
    let value = value.trim();

    // Find url(...) - take the first one if there are multiple (fallbacks)
    let url_start = value.find("url(")?;
    let after_url = &value[url_start + 4..];

    // Find the closing paren
    let url_end = after_url.find(')')?;
    let url_content = &after_url[..url_end];

    // Remove quotes if present
    let url = url_content
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();

    Some(url)
}

/// Parse a font-family value, returning the first (primary) font
/// If css_vars is provided, resolves var() references
fn parse_font_family_value(value: &str) -> String {
    parse_font_family_value_with_vars(value, &HashMap::new())
}

/// Parse a font-family value with CSS variable resolution
fn parse_font_family_value_with_vars(value: &str, css_vars: &CssVariables) -> String {
    let value = value.trim();

    // Resolve var() references first
    let resolved = resolve_css_var(value, css_vars);

    // font-family can be: "Font Name", 'Font Name', Font-Name, or a list
    // We take the first one
    let first = resolved.split(',').next().unwrap_or(&resolved).trim();

    // Remove quotes if present
    let first = first.trim_matches('"').trim_matches('\'');

    first.to_string()
}

/// Resolve CSS var() references in a value
/// Handles: var(--property-name) and var(--property-name, fallback)
fn resolve_css_var(value: &str, css_vars: &CssVariables) -> String {
    let mut result = value.to_string();

    // Keep resolving var() references until none remain (handles nested vars)
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 10; // Prevent infinite loops from circular references

    while let Some(var_start) = result.find("var(") {
        if iterations >= MAX_ITERATIONS {
            break;
        }
        iterations += 1;

        // Find matching closing paren (handle nested parens)
        let after_var = &result[var_start + 4..];
        let mut depth = 1;
        let mut var_end = None;
        for (i, c) in after_var.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        var_end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(end_offset) = var_end else {
            break; // Malformed var()
        };

        let var_content = &after_var[..end_offset];
        let full_var_end = var_start + 4 + end_offset + 1; // Include closing paren

        // Parse var content: --property-name or --property-name, fallback
        let (var_name, fallback) = if let Some(comma_pos) = var_content.find(',') {
            let name = var_content[..comma_pos].trim();
            let fallback = var_content[comma_pos + 1..].trim();
            (name, Some(fallback))
        } else {
            (var_content.trim(), None)
        };

        // Look up the variable value
        let replacement = css_vars
            .get(var_name)
            .map(|s| s.as_str())
            .or(fallback)
            .unwrap_or("");

        // Replace the var() with its resolved value
        result = format!(
            "{}{}{}",
            &result[..var_start],
            replacement,
            &result[full_var_end..]
        );
    }

    result
}

/// Parse CSS custom property declarations from CSS
/// Returns a map of --property-name -> value
fn parse_css_custom_properties(css: &str) -> CssVariables {
    let mut vars = HashMap::new();

    // Parse through CSS looking for custom property declarations
    let mut remaining = css;

    while let Some(brace_start) = remaining.find('{') {
        let after_brace = &remaining[brace_start + 1..];

        // Find matching closing brace
        let mut depth = 1;
        let mut block_end = None;
        for (i, c) in after_brace.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        block_end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(end) = block_end else {
            break;
        };

        let block = &after_brace[..end];

        // Parse declarations in this block
        for declaration in block.split(';') {
            let declaration = declaration.trim();

            // Look for custom property declarations (--name: value)
            if declaration.starts_with("--")
                && let Some(colon_pos) = declaration.find(':')
            {
                let name = declaration[..colon_pos].trim();
                let value = declaration[colon_pos + 1..].trim();
                vars.insert(name.to_string(), value.to_string());
            }
        }

        remaining = &after_brace[end + 1..];
    }

    vars
}

/// Find which font-family applies to an element based on CSS rules
fn find_font_family_for_element(
    element: &scraper::ElementRef,
    rules: &[FontFamilyRule],
) -> Option<String> {
    let mut matched_font: Option<String> = None;

    // Check each rule (later rules override earlier ones - simplified specificity)
    for rule in rules {
        if let Ok(selector) = Selector::parse(&rule.selector) {
            // Check if this element matches the selector
            if selector.matches(element) {
                matched_font = Some(rule.font_family.clone());
            }
        }
    }

    // If no direct match, check ancestors (font-family is inherited)
    if matched_font.is_none() {
        for ancestor in element.ancestors() {
            if let Some(ancestor_el) = scraper::ElementRef::wrap(ancestor) {
                for rule in rules {
                    if let Ok(selector) = Selector::parse(&rule.selector)
                        && selector.matches(&ancestor_el)
                    {
                        matched_font = Some(rule.font_family.clone());
                        // Don't break - later rules still override
                    }
                }
            }
            if matched_font.is_some() {
                break;
            }
        }
    }

    matched_font
}

/// Extract CSS from HTML document (from `<style>` tags and inline styles)
pub fn extract_css_from_html(html: &str) -> String {
    let document = Html::parse_document(html);
    let style_selector = Selector::parse("style").unwrap();

    let mut css = String::new();

    for style in document.select(&style_selector) {
        css.push_str(&style.inner_html());
        css.push('\n');
    }

    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_font_family_rules() {
        let css = r#"
            body { font-family: "Inter", sans-serif; }
            h1 { font-family: 'Playfair Display'; }
            .code { font-family: monospace; }
        "#;

        let rules = parse_font_family_rules_with_vars(css, &HashMap::new());
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, "body");
        assert_eq!(rules[0].font_family, "Inter");
        assert_eq!(rules[1].font_family, "Playfair Display");
        assert_eq!(rules[2].font_family, "monospace");
    }

    #[test]
    fn test_collect_chars_basic() {
        let html = r#"
            <html>
            <head>
                <style>
                    body { font-family: "TestFont"; }
                </style>
            </head>
            <body>
                <p>Hello</p>
            </body>
            </html>
        "#;

        let css = extract_css_from_html(html);
        let chars = collect_chars_per_font(html, &css);

        assert!(chars.contains_key("TestFont"));
        let test_font_chars = &chars["TestFont"];
        assert!(test_font_chars.contains(&'H'));
        assert!(test_font_chars.contains(&'e'));
        assert!(test_font_chars.contains(&'l'));
        assert!(test_font_chars.contains(&'o'));
    }

    #[test]
    fn test_different_fonts_for_elements() {
        let html = r#"
            <html>
            <head>
                <style>
                    body { font-family: "BodyFont"; }
                    h1 { font-family: "HeadingFont"; }
                </style>
            </head>
            <body>
                <h1>Title</h1>
                <p>Body text</p>
            </body>
            </html>
        "#;

        let css = extract_css_from_html(html);
        let chars = collect_chars_per_font(html, &css);

        // h1 should use HeadingFont
        assert!(chars.contains_key("HeadingFont"));
        assert!(chars["HeadingFont"].contains(&'T'));

        // p should inherit from body -> BodyFont
        assert!(chars.contains_key("BodyFont"));
        assert!(chars["BodyFont"].contains(&'B'));
    }

    #[test]
    fn test_parse_font_face_rules() {
        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/Inter-Regular.woff2") format("woff2");
                font-weight: 400;
                font-style: normal;
            }

            @font-face {
                font-family: "Inter";
                src: url('/fonts/Inter-Bold.woff2');
                font-weight: 700;
            }

            @font-face {
                font-family: 'Playfair Display';
                src: url(fonts/Playfair.ttf);
            }

            body { font-family: "Inter", sans-serif; }
        "#;

        let faces = parse_font_face_rules(css);
        assert_eq!(faces.len(), 3);

        assert_eq!(faces[0].family, "Inter");
        assert_eq!(faces[0].src, "/fonts/Inter-Regular.woff2");
        assert_eq!(faces[0].weight, Some("400".to_string()));
        assert_eq!(faces[0].style, Some("normal".to_string()));

        assert_eq!(faces[1].family, "Inter");
        assert_eq!(faces[1].src, "/fonts/Inter-Bold.woff2");
        assert_eq!(faces[1].weight, Some("700".to_string()));
        assert_eq!(faces[1].style, None);

        assert_eq!(faces[2].family, "Playfair Display");
        assert_eq!(faces[2].src, "fonts/Playfair.ttf");
    }

    #[test]
    fn test_analyze_fonts_full() {
        let html = r#"
            <html>
            <head>
                <style>
                    @font-face {
                        font-family: "MyFont";
                        src: url("/fonts/MyFont.woff2");
                    }
                    body { font-family: "MyFont"; }
                </style>
            </head>
            <body>
                <p>Hello World</p>
            </body>
            </html>
        "#;

        let css = extract_css_from_html(html);
        let analysis = analyze_fonts(html, &css);

        // Should have the font-face
        assert_eq!(analysis.font_faces.len(), 1);
        assert_eq!(analysis.font_faces[0].family, "MyFont");
        assert_eq!(analysis.font_faces[0].src, "/fonts/MyFont.woff2");

        // Should have collected chars for MyFont
        assert!(analysis.chars_per_font.contains_key("MyFont"));
        let chars = &analysis.chars_per_font["MyFont"];
        assert!(chars.contains(&'H'));
        assert!(chars.contains(&'W'));
    }

    #[test]
    fn test_parse_css_custom_properties() {
        let css = r#"
            :root {
                --font-mono: 'Iosevka', monospace;
                --font-body: "Inter", sans-serif;
                --spacing: 1rem;
            }
            body { color: black; }
        "#;

        let vars = parse_css_custom_properties(css);
        assert_eq!(vars.get("--font-mono"), Some(&"'Iosevka', monospace".to_string()));
        assert_eq!(vars.get("--font-body"), Some(&"\"Inter\", sans-serif".to_string()));
        assert_eq!(vars.get("--spacing"), Some(&"1rem".to_string()));
    }

    #[test]
    fn test_resolve_css_var_simple() {
        let mut vars = HashMap::new();
        vars.insert("--font-mono".to_string(), "'Iosevka', monospace".to_string());

        let result = resolve_css_var("var(--font-mono)", &vars);
        assert_eq!(result, "'Iosevka', monospace");
    }

    #[test]
    fn test_resolve_css_var_with_fallback() {
        let vars: CssVariables = HashMap::new();

        // When variable doesn't exist, should use fallback
        let result = resolve_css_var("var(--undefined, Arial)", &vars);
        assert_eq!(result, "Arial");
    }

    #[test]
    fn test_resolve_css_var_nested() {
        let mut vars = HashMap::new();
        vars.insert("--base-font".to_string(), "'Inter'".to_string());
        vars.insert("--font-stack".to_string(), "var(--base-font), sans-serif".to_string());

        let result = resolve_css_var("var(--font-stack)", &vars);
        assert_eq!(result, "'Inter', sans-serif");
    }

    #[test]
    fn test_font_family_with_css_var() {
        // This is the exact reproduction case from the issue
        let html = r#"
            <html>
            <head>
                <style>
                    @font-face {
                        font-family: 'Iosevka';
                        src: url('/fonts/Iosevka-Regular.woff2') format('woff2');
                    }

                    :root {
                        --font-mono: 'Iosevka', monospace;
                    }

                    code {
                        font-family: var(--font-mono);
                    }
                </style>
            </head>
            <body>
                <code>fn main() { println!("hello"); }</code>
            </body>
            </html>
        "#;

        let css = extract_css_from_html(html);
        let analysis = analyze_fonts(html, &css);

        // Should have the font-face for Iosevka
        assert_eq!(analysis.font_faces.len(), 1);
        assert_eq!(analysis.font_faces[0].family, "Iosevka");

        // Should have collected chars for Iosevka (not None/empty!)
        assert!(analysis.chars_per_font.contains_key("Iosevka"),
            "chars_per_font should contain Iosevka, but got: {:?}",
            analysis.chars_per_font.keys().collect::<Vec<_>>());

        let iosevka_chars = &analysis.chars_per_font["Iosevka"];
        // Check for characters from: fn main() { println!("hello"); }
        assert!(iosevka_chars.contains(&'f'));
        assert!(iosevka_chars.contains(&'n'));
        assert!(iosevka_chars.contains(&'m'));
        assert!(iosevka_chars.contains(&'('));
        assert!(iosevka_chars.contains(&'{'));
        assert!(iosevka_chars.contains(&'h'));
        assert!(iosevka_chars.contains(&'e'));
        assert!(iosevka_chars.contains(&'l'));
        assert!(iosevka_chars.contains(&'o'));
    }

    #[test]
    fn test_css_var_in_multiple_rules() {
        let css = r#"
            :root {
                --heading-font: 'Playfair Display';
                --body-font: 'Inter';
            }

            h1 { font-family: var(--heading-font); }
            h2 { font-family: var(--heading-font); }
            p { font-family: var(--body-font); }
        "#;

        let vars = parse_css_custom_properties(css);
        let rules = parse_font_family_rules_with_vars(css, &vars);

        // Should have 3 rules (h1, h2, p)
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].font_family, "Playfair Display");
        assert_eq!(rules[1].font_family, "Playfair Display");
        assert_eq!(rules[2].font_family, "Inter");
    }
}
