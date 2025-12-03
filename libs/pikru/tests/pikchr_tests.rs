use datatest_stable::Utf8Path;
use std::collections::HashMap;
use std::process::Command;

/// Path to the C pikchr binary (built from ../pikchr)
const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../pikchr/pikchr");

/// Tolerance for floating-point comparisons (pikchr uses single precision)
const FLOAT_TOLERANCE: f64 = 0.01;

// =============================================================================
// SVG Comparison Types
// =============================================================================

/// A parsed SVG element for comparison
#[derive(Debug, Clone)]
struct SvgElement {
    tag: String,
    /// Position (for matching elements)
    pos: Option<(f64, f64)>,
    /// All numeric attributes
    attrs: HashMap<String, f64>,
    /// All string attributes (fill, stroke colors, etc.)
    str_attrs: HashMap<String, String>,
    /// Text content for text elements
    text_content: Option<String>,
    /// Parsed path commands for path elements
    path_commands: Vec<PathCommand>,
}

/// A single path command (M, L, C, Q, A, etc.)
#[derive(Debug, Clone)]
struct PathCommand {
    cmd: char,
    args: Vec<f64>,
}

// =============================================================================
// SVG Parsing
// =============================================================================

/// Parse an SVG string into a list of comparable elements
fn parse_svg(svg: &str) -> Result<Vec<SvgElement>, String> {
    let doc = roxmltree::Document::parse(svg).map_err(|e| format!("XML parse error: {}", e))?;

    let mut elements = Vec::new();

    for node in doc.descendants() {
        if !node.is_element() {
            continue;
        }

        let tag = node.tag_name().name();

        // Skip container elements
        if matches!(tag, "svg" | "g" | "defs" | "style") {
            continue;
        }

        let element = match tag {
            "rect" => parse_rect(&node),
            "circle" => parse_circle(&node),
            "ellipse" => parse_ellipse(&node),
            "line" => parse_line(&node),
            "path" => parse_path(&node),
            "polygon" => parse_polygon(&node),
            "polyline" => parse_polyline(&node),
            "text" => parse_text(&node),
            _ => continue,
        };

        elements.push(element);
    }

    Ok(elements)
}

fn parse_rect(node: &roxmltree::Node) -> SvgElement {
    let mut attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    for attr in ["x", "y", "width", "height", "rx", "ry"] {
        if let Some(v) = get_num_attr(node, attr) {
            attrs.insert(attr.to_string(), v);
        }
    }

    parse_style_attrs(node, &mut str_attrs);

    let x = attrs.get("x").copied();
    let y = attrs.get("y").copied();
    let pos = match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    SvgElement {
        tag: "rect".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands: vec![],
    }
}

fn parse_circle(node: &roxmltree::Node) -> SvgElement {
    let mut attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    for attr in ["cx", "cy", "r"] {
        if let Some(v) = get_num_attr(node, attr) {
            attrs.insert(attr.to_string(), v);
        }
    }

    parse_style_attrs(node, &mut str_attrs);

    let cx = attrs.get("cx").copied();
    let cy = attrs.get("cy").copied();
    let pos = match (cx, cy) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    SvgElement {
        tag: "circle".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands: vec![],
    }
}

fn parse_ellipse(node: &roxmltree::Node) -> SvgElement {
    let mut attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    for attr in ["cx", "cy", "rx", "ry"] {
        if let Some(v) = get_num_attr(node, attr) {
            attrs.insert(attr.to_string(), v);
        }
    }

    parse_style_attrs(node, &mut str_attrs);

    let cx = attrs.get("cx").copied();
    let cy = attrs.get("cy").copied();
    let pos = match (cx, cy) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    SvgElement {
        tag: "ellipse".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands: vec![],
    }
}

fn parse_line(node: &roxmltree::Node) -> SvgElement {
    let mut attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    for attr in ["x1", "y1", "x2", "y2"] {
        if let Some(v) = get_num_attr(node, attr) {
            attrs.insert(attr.to_string(), v);
        }
    }

    parse_style_attrs(node, &mut str_attrs);

    let x1 = attrs.get("x1").copied();
    let y1 = attrs.get("y1").copied();
    let pos = match (x1, y1) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    SvgElement {
        tag: "line".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands: vec![],
    }
}

fn parse_path(node: &roxmltree::Node) -> SvgElement {
    let attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    parse_style_attrs(node, &mut str_attrs);

    let path_commands = if let Some(d) = node.attribute("d") {
        parse_path_data(d)
    } else {
        vec![]
    };

    // Use first command's position as element position
    let pos = path_commands.first().and_then(|cmd| {
        if cmd.args.len() >= 2 {
            Some((cmd.args[0], cmd.args[1]))
        } else {
            None
        }
    });

    SvgElement {
        tag: "path".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands,
    }
}

fn parse_polygon(node: &roxmltree::Node) -> SvgElement {
    let attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    parse_style_attrs(node, &mut str_attrs);

    // Parse points attribute
    let points = if let Some(p) = node.attribute("points") {
        parse_points(p)
    } else {
        vec![]
    };

    // Convert points to path commands for easier comparison
    let path_commands: Vec<PathCommand> = points
        .iter()
        .enumerate()
        .map(|(i, (x, y))| PathCommand {
            cmd: if i == 0 { 'M' } else { 'L' },
            args: vec![*x, *y],
        })
        .collect();

    let pos = points.first().copied();

    SvgElement {
        tag: "polygon".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: None,
        path_commands,
    }
}

fn parse_polyline(node: &roxmltree::Node) -> SvgElement {
    let mut el = parse_polygon(node);
    el.tag = "polyline".to_string();
    el
}

fn parse_text(node: &roxmltree::Node) -> SvgElement {
    let mut attrs = HashMap::new();
    let mut str_attrs = HashMap::new();

    for attr in ["x", "y"] {
        if let Some(v) = get_num_attr(node, attr) {
            attrs.insert(attr.to_string(), v);
        }
    }

    parse_style_attrs(node, &mut str_attrs);

    // Get text content
    let text_content: String = node
        .descendants()
        .filter(|n| n.is_text())
        .map(|n| n.text().unwrap_or(""))
        .collect();

    let x = attrs.get("x").copied();
    let y = attrs.get("y").copied();
    let pos = match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };

    SvgElement {
        tag: "text".to_string(),
        pos,
        attrs,
        str_attrs,
        text_content: Some(text_content),
        path_commands: vec![],
    }
}

fn get_num_attr(node: &roxmltree::Node, name: &str) -> Option<f64> {
    node.attribute(name).and_then(|v| v.parse().ok())
}

fn parse_style_attrs(node: &roxmltree::Node, str_attrs: &mut HashMap<String, String>) {
    for attr in [
        "fill",
        "stroke",
        "stroke-width",
        "stroke-dasharray",
        "style",
    ] {
        if let Some(v) = node.attribute(attr) {
            str_attrs.insert(attr.to_string(), normalize_color(v));
        }
    }
}

/// Normalize color values to a common format
fn normalize_color(color: &str) -> String {
    let color = color.trim();

    // Handle "none"
    if color.eq_ignore_ascii_case("none") {
        return "none".to_string();
    }

    // Handle rgb() format -> convert to hex
    if color.starts_with("rgb(") {
        if let Some(hex) = rgb_to_hex(color) {
            return hex;
        }
    }

    color.to_lowercase()
}

fn rgb_to_hex(rgb: &str) -> Option<String> {
    let inner = rgb.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let r: u8 = parts[0].trim().parse().ok()?;
    let g: u8 = parts[1].trim().parse().ok()?;
    let b: u8 = parts[2].trim().parse().ok()?;
    Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
}

// =============================================================================
// Path Data Parsing
// =============================================================================

fn parse_path_data(d: &str) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    let mut chars = d.chars().peekable();
    let mut current_cmd = 'M';

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() || c == ',' {
            chars.next();
            continue;
        }

        if c.is_ascii_alphabetic() {
            current_cmd = c;
            chars.next();
            continue;
        }

        // Parse numbers for this command
        let arg_count = path_cmd_arg_count(current_cmd);
        let mut args = Vec::new();

        for _ in 0..arg_count {
            // Skip whitespace and commas
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == ',' {
                    chars.next();
                } else {
                    break;
                }
            }

            if let Some(num) = parse_number(&mut chars) {
                args.push(num);
            } else {
                break;
            }
        }

        if !args.is_empty() {
            commands.push(PathCommand {
                cmd: current_cmd.to_ascii_uppercase(),
                args,
            });
        }
    }

    commands
}

fn path_cmd_arg_count(cmd: char) -> usize {
    match cmd.to_ascii_uppercase() {
        'M' | 'L' | 'T' => 2,
        'H' | 'V' => 1,
        'C' => 6,
        'S' | 'Q' => 4,
        'A' => 7,
        'Z' => 0,
        _ => 2,
    }
}

fn parse_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<f64> {
    let mut s = String::new();

    // Handle negative sign
    if chars.peek() == Some(&'-') {
        s.push(chars.next().unwrap());
    }

    // Collect digits and decimal point
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '.' {
            s.push(chars.next().unwrap());
        } else {
            break;
        }
    }

    // Handle exponent
    if chars.peek() == Some(&'e') || chars.peek() == Some(&'E') {
        s.push(chars.next().unwrap());
        if chars.peek() == Some(&'-') || chars.peek() == Some(&'+') {
            s.push(chars.next().unwrap());
        }
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                s.push(chars.next().unwrap());
            } else {
                break;
            }
        }
    }

    if s.is_empty() || s == "-" {
        None
    } else {
        s.parse().ok()
    }
}

fn parse_points(points: &str) -> Vec<(f64, f64)> {
    let mut result = Vec::new();
    let mut chars = points.chars().peekable();

    loop {
        // Skip whitespace and commas
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ',' {
                chars.next();
            } else {
                break;
            }
        }

        let x = match parse_number(&mut chars) {
            Some(n) => n,
            None => break,
        };

        // Skip separator
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ',' {
                chars.next();
            } else {
                break;
            }
        }

        let y = match parse_number(&mut chars) {
            Some(n) => n,
            None => break,
        };

        result.push((x, y));
    }

    result
}

// =============================================================================
// SVG Comparison
// =============================================================================

/// Result of comparing two SVG documents
#[derive(Debug)]
struct SvgDiff {
    /// Elements only in the first SVG (C version)
    only_in_c: Vec<SvgElement>,
    /// Elements only in the second SVG (Rust version)
    only_in_rust: Vec<SvgElement>,
    /// Pairs of matched elements with differences
    differences: Vec<ElementDiff>,
}

#[derive(Debug)]
struct ElementDiff {
    c_element: SvgElement,
    rust_element: SvgElement,
    attr_diffs: Vec<AttrDiff>,
}

#[derive(Debug)]
enum AttrDiff {
    NumericDiff {
        name: String,
        c_val: f64,
        rust_val: f64,
    },
    StringDiff {
        name: String,
        c_val: String,
        rust_val: String,
    },
    TextContentDiff {
        c_val: String,
        rust_val: String,
    },
    PathDiff {
        detail: String,
    },
    Missing {
        name: String,
        in_c: bool,
    },
}

fn compare_svgs(c_svg: &str, rust_svg: &str) -> Result<SvgDiff, String> {
    let c_elements = parse_svg(c_svg)?;
    let rust_elements = parse_svg(rust_svg)?;

    let mut matched_c: Vec<bool> = vec![false; c_elements.len()];
    let mut matched_rust: Vec<bool> = vec![false; rust_elements.len()];
    let mut differences = Vec::new();

    // Match elements by tag and position
    for (ci, c_el) in c_elements.iter().enumerate() {
        if let Some((ri, r_el)) = find_best_match(c_el, &rust_elements, &matched_rust) {
            matched_c[ci] = true;
            matched_rust[ri] = true;

            // Compare the matched elements
            let attr_diffs = compare_elements(c_el, r_el);
            if !attr_diffs.is_empty() {
                differences.push(ElementDiff {
                    c_element: c_el.clone(),
                    rust_element: r_el.clone(),
                    attr_diffs,
                });
            }
        }
    }

    let only_in_c: Vec<SvgElement> = c_elements
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !matched_c[*i])
        .map(|(_, e)| e)
        .collect();

    let only_in_rust: Vec<SvgElement> = rust_elements
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !matched_rust[*i])
        .map(|(_, e)| e)
        .collect();

    Ok(SvgDiff {
        only_in_c,
        only_in_rust,
        differences,
    })
}

fn find_best_match<'a>(
    target: &SvgElement,
    candidates: &'a [SvgElement],
    used: &[bool],
) -> Option<(usize, &'a SvgElement)> {
    let mut best_match: Option<(usize, &SvgElement, f64)> = None;

    for (i, candidate) in candidates.iter().enumerate() {
        if used[i] {
            continue;
        }

        // Must be same tag
        if target.tag != candidate.tag {
            continue;
        }

        // Calculate position distance
        let distance = match (target.pos, candidate.pos) {
            (Some((x1, y1)), Some((x2, y2))) => ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt(),
            (None, None) => 0.0, // Both have no position, consider them close
            _ => f64::MAX,       // One has position, other doesn't
        };

        // Accept match if within reasonable distance (10 units)
        if distance < 10.0 {
            if let Some((_, _, best_dist)) = best_match {
                if distance < best_dist {
                    best_match = Some((i, candidate, distance));
                }
            } else {
                best_match = Some((i, candidate, distance));
            }
        }
    }

    best_match.map(|(i, el, _)| (i, el))
}

fn compare_elements(c_el: &SvgElement, r_el: &SvgElement) -> Vec<AttrDiff> {
    let mut diffs = Vec::new();

    // Compare numeric attributes
    for (name, &c_val) in &c_el.attrs {
        if let Some(&r_val) = r_el.attrs.get(name) {
            if !floats_equal(c_val, r_val) {
                diffs.push(AttrDiff::NumericDiff {
                    name: name.clone(),
                    c_val,
                    rust_val: r_val,
                });
            }
        } else {
            diffs.push(AttrDiff::Missing {
                name: name.clone(),
                in_c: true,
            });
        }
    }

    // Check for attrs only in Rust
    for name in r_el.attrs.keys() {
        if !c_el.attrs.contains_key(name) {
            diffs.push(AttrDiff::Missing {
                name: name.clone(),
                in_c: false,
            });
        }
    }

    // Compare string attributes
    for (name, c_val) in &c_el.str_attrs {
        if let Some(r_val) = r_el.str_attrs.get(name) {
            if c_val != r_val {
                diffs.push(AttrDiff::StringDiff {
                    name: name.clone(),
                    c_val: c_val.clone(),
                    rust_val: r_val.clone(),
                });
            }
        }
        // Don't report missing string attrs as errors (style handling varies)
    }

    // Compare text content
    if let (Some(c_text), Some(r_text)) = (&c_el.text_content, &r_el.text_content) {
        if c_text.trim() != r_text.trim() {
            diffs.push(AttrDiff::TextContentDiff {
                c_val: c_text.clone(),
                rust_val: r_text.clone(),
            });
        }
    }

    // Compare path commands
    if !c_el.path_commands.is_empty() || !r_el.path_commands.is_empty() {
        if let Some(diff) = compare_paths(&c_el.path_commands, &r_el.path_commands) {
            diffs.push(AttrDiff::PathDiff { detail: diff });
        }
    }

    diffs
}

fn compare_paths(c_cmds: &[PathCommand], r_cmds: &[PathCommand]) -> Option<String> {
    if c_cmds.len() != r_cmds.len() {
        return Some(format!(
            "Command count differs: C has {}, Rust has {}",
            c_cmds.len(),
            r_cmds.len()
        ));
    }

    for (i, (c_cmd, r_cmd)) in c_cmds.iter().zip(r_cmds.iter()).enumerate() {
        if c_cmd.cmd != r_cmd.cmd {
            return Some(format!(
                "Command {} differs: C='{}', Rust='{}'",
                i, c_cmd.cmd, r_cmd.cmd
            ));
        }

        if c_cmd.args.len() != r_cmd.args.len() {
            return Some(format!(
                "Command {} '{}' arg count differs: C has {}, Rust has {}",
                i,
                c_cmd.cmd,
                c_cmd.args.len(),
                r_cmd.args.len()
            ));
        }

        for (j, (c_arg, r_arg)) in c_cmd.args.iter().zip(r_cmd.args.iter()).enumerate() {
            if !floats_equal(*c_arg, *r_arg) {
                return Some(format!(
                    "Command {} '{}' arg {} differs: C={}, Rust={}",
                    i, c_cmd.cmd, j, c_arg, r_arg
                ));
            }
        }
    }

    None
}

fn floats_equal(a: f64, b: f64) -> bool {
    (a - b).abs() < FLOAT_TOLERANCE
}

impl SvgDiff {
    fn is_empty(&self) -> bool {
        self.only_in_c.is_empty() && self.only_in_rust.is_empty() && self.differences.is_empty()
    }

    fn format_report(&self) -> String {
        let mut report = String::new();

        if !self.only_in_c.is_empty() {
            report.push_str("Elements only in C output:\n");
            for el in &self.only_in_c {
                report.push_str(&format!("  - {} at {:?}\n", el.tag, el.pos));
            }
        }

        if !self.only_in_rust.is_empty() {
            report.push_str("Elements only in Rust output:\n");
            for el in &self.only_in_rust {
                report.push_str(&format!("  - {} at {:?}\n", el.tag, el.pos));
            }
        }

        if !self.differences.is_empty() {
            report.push_str("Element differences:\n");
            for diff in &self.differences {
                report.push_str(&format!(
                    "  {} at C={:?} / Rust={:?}:\n",
                    diff.c_element.tag, diff.c_element.pos, diff.rust_element.pos
                ));
                for attr_diff in &diff.attr_diffs {
                    match attr_diff {
                        AttrDiff::NumericDiff {
                            name,
                            c_val,
                            rust_val,
                        } => {
                            report.push_str(&format!(
                                "    {}: C={:.4}, Rust={:.4}\n",
                                name, c_val, rust_val
                            ));
                        }
                        AttrDiff::StringDiff {
                            name,
                            c_val,
                            rust_val,
                        } => {
                            report.push_str(&format!(
                                "    {}: C='{}', Rust='{}'\n",
                                name, c_val, rust_val
                            ));
                        }
                        AttrDiff::TextContentDiff { c_val, rust_val } => {
                            report.push_str(&format!(
                                "    text: C='{}', Rust='{}'\n",
                                c_val, rust_val
                            ));
                        }
                        AttrDiff::PathDiff { detail } => {
                            report.push_str(&format!("    path: {}\n", detail));
                        }
                        AttrDiff::Missing { name, in_c } => {
                            if *in_c {
                                report.push_str(&format!("    {}: missing in Rust\n", name));
                            } else {
                                report.push_str(&format!("    {}: missing in C\n", name));
                            }
                        }
                    }
                }
            }
        }

        report
    }
}

/// Run the C pikchr implementation and return its SVG output
fn run_c_pikchr(source: &str) -> String {
    let mut child = Command::new(C_PIKCHR)
        .arg("--svg-only")
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run C pikchr");

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = child
        .wait_with_output()
        .expect("failed to wait on C pikchr");
    String::from_utf8(output.stdout).expect("C pikchr output not UTF-8")
}

fn test_pikchr_file(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = std::fs::read_to_string(path)?;

    // Get expected output from C implementation
    let c_output = run_c_pikchr(&source);

    // Get output from our Rust implementation
    let rust_result = pikru::pikchr(&source);

    match rust_result {
        Ok(rust_output) => {
            // Semantic comparison using DOM parsing
            match compare_svgs(&c_output, &rust_output) {
                Ok(diff) => {
                    if !diff.is_empty() {
                        panic!(
                            "SVG mismatch for {}:\n{}\n\n--- C output ---\n{}\n\n--- Rust output ---\n{}",
                            path,
                            diff.format_report(),
                            c_output,
                            rust_output
                        );
                    }
                }
                Err(e) => {
                    panic!("Failed to parse SVG for comparison: {}", e);
                }
            }
        }
        Err(e) => {
            // For now, just note that Rust implementation isn't done yet
            panic!("Rust implementation failed for {}: {}", path, e);
        }
    }

    Ok(())
}

/// Compare two pikchr sources semantically
#[allow(dead_code)]
fn assert_svg_matches(source: &str, context: &str) {
    let c_out = run_c_pikchr(source);
    let rust_out = pikru::pikchr(source).expect("rust pikchr failed");

    match compare_svgs(&c_out, &rust_out) {
        Ok(diff) => {
            if !diff.is_empty() {
                panic!(
                    "SVG mismatch ({}):\n{}\n\n--- C output ---\n{}\n\n--- Rust output ---\n{}",
                    context,
                    diff.format_report(),
                    c_out,
                    rust_out
                );
            }
        }
        Err(e) => {
            panic!("Failed to parse SVG: {}", e);
        }
    }
}

#[test]
fn even_with_horizontal_matches_c() {
    let source = r#"
        box "A"
        B: box at (2,1)
        arrow right even with B
    "#;
    assert_svg_matches(source, "right even with should align x to target");
}

#[test]
fn until_even_vertical_matches_c() {
    let source = r#"
        box "A"
        B: box at (1,2)
        arrow down until even with B
    "#;
    assert_svg_matches(source, "until even with should align y to target");
}

datatest_stable::harness! {
    { test = test_pikchr_file, root = concat!(env!("CARGO_MANIFEST_DIR"), "/../pikchr/tests"), pattern = r"\.pikchr$" },
}
