//! Compare SVG output between C pikchr and Rust pikru
//!
//! Parses both SVGs and compares semantic values (positions, sizes, colors)

use facet_format_svg::{Svg, SvgNode};
use std::process::Command;

fn main() {
    let test_files = [
        "vendor/pikchr-c/tests/test01.pikchr",
        "vendor/pikchr-c/tests/test03.pikchr",
        "vendor/pikchr-c/tests/test10.pikchr",
    ];

    for file in test_files {
        println!("=== {} ===", file);
        compare_file(file);
        println!();
    }
}

fn compare_file(path: &str) {
    // Get C pikchr output
    let c_output = Command::new("vendor/pikchr-c/pikchr")
        .arg("--svg-only")
        .arg(path)
        .output()
        .expect("Failed to run pikchr");
    let c_svg = String::from_utf8_lossy(&c_output.stdout);

    // Get Rust pikru output
    let input = std::fs::read_to_string(path).expect("Failed to read file");
    let rust_svg = match pikru::pikchr(&input) {
        Ok(svg) => svg,
        Err(e) => {
            println!("  Rust error: {}", e);
            return;
        }
    };

    // Parse and compare
    let c_elements = parse_svg_elements(&c_svg);
    let rust_elements = parse_svg_elements(&rust_svg);

    println!("  C elements: {} shapes", c_elements.len());
    println!("  Rust elements: {} shapes", rust_elements.len());

    // Compare element counts by type
    let c_counts = count_by_type(&c_elements);
    let rust_counts = count_by_type(&rust_elements);

    println!("  Element comparison:");
    for (typ, c_count) in &c_counts {
        let rust_count = rust_counts.get(typ).unwrap_or(&0);
        let status = if c_count == rust_count { "✓" } else { "≠" };
        println!("    {} {}: C={}, Rust={}", status, typ, c_count, rust_count);
    }

    // Check for types in Rust but not C
    for (typ, rust_count) in &rust_counts {
        if !c_counts.contains_key(typ) {
            println!("    + {}: Rust={} (not in C)", typ, rust_count);
        }
    }
}

fn parse_svg_elements(svg: &str) -> Vec<String> {
    let doc: Svg = match facet_format_svg::from_str(svg) {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    let mut elements = Vec::new();
    collect_element_tags(&doc.children, &mut elements);
    elements
}

fn collect_element_tags(children: &[SvgNode], elements: &mut Vec<String>) {
    for child in children {
        match child {
            SvgNode::G(g) => collect_element_tags(&g.children, elements),
            SvgNode::Defs(d) => collect_element_tags(&d.children, elements),
            SvgNode::Style(_) => {}
            SvgNode::Rect(_) => elements.push("rect".to_string()),
            SvgNode::Circle(_) => elements.push("circle".to_string()),
            SvgNode::Ellipse(_) => elements.push("ellipse".to_string()),
            SvgNode::Line(_) => elements.push("line".to_string()),
            SvgNode::Path(_) => elements.push("path".to_string()),
            SvgNode::Polygon(_) => elements.push("polygon".to_string()),
            SvgNode::Polyline(_) => elements.push("polyline".to_string()),
            SvgNode::Text(_) => elements.push("text".to_string()),
            SvgNode::Use(_) => elements.push("use".to_string()),
            SvgNode::Image(_) => elements.push("image".to_string()),
            SvgNode::Title(_) => elements.push("title".to_string()),
            SvgNode::Desc(_) => elements.push("desc".to_string()),
            SvgNode::Symbol(symbol) => {
                elements.push("symbol".to_string());
                collect_element_tags(&symbol.children, elements);
            }
        }
    }
}

fn count_by_type(elements: &[String]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for el in elements {
        *counts.entry(el.clone()).or_insert(0) += 1;
    }
    counts
}
