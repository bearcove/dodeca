//! JavaScript string literal rewriting plugin for dodeca
//!
//! Uses OXC parser to find and rewrite string literals containing asset paths.

use facet::Facet;
use oxc::allocator::Allocator;
use oxc::ast::ast::{StringLiteral, TemplateLiteral};
use oxc::ast_visit::Visit;
use oxc::parser::Parser;
use oxc::span::SourceType;
use plugcard::{plugcard, PlugResult};
use std::collections::HashMap;

plugcard::export_plugin!();

/// Input for JS string literal rewriting
#[derive(Facet)]
pub struct JsRewriteInput {
    /// The JavaScript source code
    pub js: String,
    /// Map of old paths to new paths
    pub path_map: HashMap<String, String>,
}

/// Rewrite string literals in JavaScript that contain asset paths
///
/// Parses JavaScript, finds string literals matching paths in path_map,
/// and replaces them with the new paths.
#[plugcard]
pub fn rewrite_string_literals_in_js(input: JsRewriteInput) -> PlugResult<String> {
    let js = &input.js;
    let path_map = &input.path_map;

    // Parse the JavaScript
    let allocator = Allocator::default();
    let source_type = SourceType::mjs(); // Treat as ES module
    let parser_result = Parser::new(&allocator, js, source_type).parse();

    if parser_result.panicked || !parser_result.errors.is_empty() {
        // If parsing fails, return unchanged (could be a snippet or invalid JS)
        return PlugResult::Ok(js.to_string());
    }

    // Collect string literal positions and their replacement values
    let mut replacements: Vec<(u32, u32, String)> = Vec::new(); // (start, end, new_value)
    let mut collector = StringCollector {
        source: js,
        path_map,
        replacements: &mut replacements,
    };
    collector.visit_program(&parser_result.program);

    // Apply replacements in reverse order (so offsets stay valid)
    if replacements.is_empty() {
        return PlugResult::Ok(js.to_string());
    }

    replacements.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by start position, descending

    let mut result = js.to_string();
    for (start, end, new_value) in replacements {
        result.replace_range(start as usize..end as usize, &new_value);
    }

    PlugResult::Ok(result)
}

/// Visitor that collects string literals for replacement
struct StringCollector<'a> {
    source: &'a str,
    path_map: &'a HashMap<String, String>,
    replacements: &'a mut Vec<(u32, u32, String)>,
}

impl<'a> Visit<'_> for StringCollector<'a> {
    fn visit_string_literal(&mut self, lit: &StringLiteral<'_>) {
        let value = lit.value.as_str();
        let mut new_value = value.to_string();
        let mut changed = false;

        for (old_path, new_path) in self.path_map.iter() {
            if new_value.contains(old_path.as_str()) {
                new_value = new_value.replace(old_path, new_path);
                changed = true;
            }
        }

        if changed {
            // Get the original source including quotes
            let start = lit.span.start;
            let end = lit.span.end;
            let original = &self.source[start as usize..end as usize];
            let quote = original.chars().next().unwrap_or('"');
            self.replacements
                .push((start, end, format!("{quote}{new_value}{quote}")));
        }
    }

    fn visit_template_literal(&mut self, lit: &TemplateLiteral<'_>) {
        // Handle template literal quasi strings
        for quasi in &lit.quasis {
            let value = quasi.value.raw.as_str();
            let mut new_value = value.to_string();
            let mut changed = false;

            for (old_path, new_path) in self.path_map.iter() {
                if new_value.contains(old_path.as_str()) {
                    new_value = new_value.replace(old_path, new_path);
                    changed = true;
                }
            }

            if changed {
                // For template literals, we only replace the quasi part
                let start = quasi.span.start;
                let end = quasi.span.end;
                self.replacements.push((start, end, new_value));
            }
        }

        // Continue visiting expressions inside template literal
        for expr in &lit.expressions {
            self.visit_expression(expr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rewrite_string_literals() {
        let js = r#"
            const a = "/images/logo.png";
            const b = '/images/icon.svg';
            const c = `/images/hero.jpg`;
            const d = "/not/in/map.png";
        "#;

        let mut path_map = HashMap::new();
        path_map.insert(
            "/images/logo.png".to_string(),
            "/images/logo.abc.png".to_string(),
        );
        path_map.insert(
            "/images/icon.svg".to_string(),
            "/images/icon.def.svg".to_string(),
        );
        path_map.insert(
            "/images/hero.jpg".to_string(),
            "/images/hero.ghi.jpg".to_string(),
        );

        let input = JsRewriteInput {
            js: js.to_string(),
            path_map,
        };

        let result = rewrite_string_literals_in_js(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };

        assert!(output.contains("\"/images/logo.abc.png\""));
        assert!(output.contains("'/images/icon.def.svg'"));
        assert!(output.contains("`/images/hero.ghi.jpg`"));
        assert!(output.contains("\"/not/in/map.png\"")); // unchanged
    }

    #[test]
    fn test_invalid_js_returns_unchanged() {
        let js = "this is not { valid javascript";

        let input = JsRewriteInput {
            js: js.to_string(),
            path_map: HashMap::new(),
        };

        let result = rewrite_string_literals_in_js(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };

        assert_eq!(output, js);
    }

    #[test]
    fn test_empty_path_map() {
        let js = r#"const x = "/images/test.png";"#;

        let input = JsRewriteInput {
            js: js.to_string(),
            path_map: HashMap::new(),
        };

        let result = rewrite_string_literals_in_js(input);
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };

        assert_eq!(output, js);
    }
}
