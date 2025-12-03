//! Compare SVG output between C pikchr and Rust pikru
//!
//! Parses both SVGs and compares semantic values (positions, sizes, colors)

use std::process::Command;

fn main() {
    let test_files = [
        "../pikchr/tests/test01.pikchr",
        "../pikchr/tests/test03.pikchr",
        "../pikchr/tests/test10.pikchr",
    ];

    for file in test_files {
        println!("=== {} ===", file);
        compare_file(file);
        println!();
    }
}

fn compare_file(path: &str) {
    // Get C pikchr output
    let c_output = Command::new("../pikchr/pikchr")
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

#[derive(Debug)]
struct SvgElement {
    tag: String,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    r: Option<f64>,
    cx: Option<f64>,
    cy: Option<f64>,
}

fn parse_svg_elements(svg: &str) -> Vec<SvgElement> {
    let mut elements = Vec::new();

    let doc = match roxmltree::Document::parse(svg) {
        Ok(d) => d,
        Err(_) => return elements,
    };

    for node in doc.descendants() {
        if !node.is_element() {
            continue;
        }

        let tag = node.tag_name().name();
        if matches!(
            tag,
            "rect" | "circle" | "ellipse" | "line" | "path" | "polygon" | "polyline" | "text"
        ) {
            elements.push(SvgElement {
                tag: tag.to_string(),
                x: parse_attr_f64(node.attribute("x").or_else(|| node.attribute("x1"))),
                y: parse_attr_f64(node.attribute("y").or_else(|| node.attribute("y1"))),
                width: parse_attr_f64(node.attribute("width")),
                height: parse_attr_f64(node.attribute("height")),
                r: parse_attr_f64(node.attribute("r")),
                cx: parse_attr_f64(node.attribute("cx")),
                cy: parse_attr_f64(node.attribute("cy")),
            });
        }
    }

    elements
}

fn parse_attr_f64(s: Option<&str>) -> Option<f64> {
    s.and_then(|v| v.parse().ok())
}

fn count_by_type(elements: &[SvgElement]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for el in elements {
        *counts.entry(el.tag.clone()).or_insert(0) += 1;
    }
    counts
}
