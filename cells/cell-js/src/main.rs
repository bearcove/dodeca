//! Dodeca JS cell (cell-js)
//!
//! This cell handles JavaScript string literal rewriting using OXC.

use std::collections::HashMap;

use dodeca_cell_runtime::run_cell;
use oxc::allocator::Allocator;
use oxc::ast::ast::{StringLiteral, TemplateLiteral};
use oxc::ast_visit::Visit;
use oxc::parser::Parser;
use oxc::span::SourceType;

use cell_js_proto::{JsProcessor, JsProcessorDispatcher, JsRewriteInput};

/// JS processor implementation
#[derive(Clone)]
pub struct JsProcessorImpl;

impl JsProcessor for JsProcessorImpl {
    async fn rewrite_string_literals(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        input: JsRewriteInput,
    ) -> Result<String, String> {
        let js = &input.js;
        let path_map = &input.path_map;

        // Parse the JavaScript
        let allocator = Allocator::default();
        let source_type = SourceType::mjs(); // Treat as ES module
        let parser_result = Parser::new(&allocator, js, source_type).parse();

        if parser_result.panicked || !parser_result.errors.is_empty() {
            let first_error = parser_result
                .errors
                .first()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "parser panicked".to_string());
            return Err(first_error);
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
            return Ok(js.to_string());
        }

        replacements.sort_by(|a, b| b.0.cmp(&a.0)); // Sort by start position, descending

        let mut result = js.to_string();
        for (start, end, new_value) in replacements {
            result.replace_range(start as usize..end as usize, &new_value);
        }

        Ok(result)
    }
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("js", |_handle| JsProcessorDispatcher::new(JsProcessorImpl))
}
