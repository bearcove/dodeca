use pikru::compare::{compare_outputs, extract_pre_svg_text, extract_svg, CompareResult};
use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo xtask <command>");
        eprintln!("Commands:");
        eprintln!("  compare-html    Generate HTML comparison of all test outputs");
        eprintln!("  generate-pngs   Generate PNG files from SVG outputs for visual debugging");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "compare-html" => compare_html(),
        "generate-pngs" => generate_pngs(),
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn compare_html() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let c_pikchr = format!(
        "{}/vendor/pikchr-c/pikchr",
        manifest_dir.trim_end_matches("/xtask")
    );
    let tests_dir = Path::new(manifest_dir).join("../vendor/pikchr-c/tests");
    let output_path = Path::new(manifest_dir).join("../comparison.html");
    let debug_dir = Path::new(manifest_dir).join("../debug-svg");

    // Create debug directory for individual SVG files (ignored by git)
    fs::create_dir_all(&debug_dir).expect("Failed to create debug-svg directory");

    let mut entries: Vec<_> = fs::read_dir(&tests_dir)
        .expect("Failed to read tests directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "pikchr")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    // First pass: collect results
    // (name, source, c_output, rust_output, rust_is_err, compare_result)
    let mut results: Vec<(String, String, String, String, bool, CompareResult)> = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        eprintln!("Processing {}...", name_str);

        let source = fs::read_to_string(&path).unwrap_or_else(|_| String::new());

        // Run C pikchr
        let c_output = run_c_pikchr(&c_pikchr, &source);

        // Run Rust pikchr
        let rust_result = pikru::pikchr(&source);
        let (rust_output, rust_is_err) = match rust_result {
            Ok(s) => (s, false),
            Err(e) => (format!("Error: {}", e), true),
        };

        // Use shared comparison logic
        let compare_result = compare_outputs(&c_output, &rust_output, rust_is_err);

        // Write individual SVG files for debugging
        let c_svg = extract_svg(&c_output).unwrap_or("<!-- No SVG found -->");
        let rust_svg = extract_svg(&rust_output).unwrap_or("<!-- No SVG found -->");

        let base_name = name_str.trim_end_matches(".pikchr");
        let c_file = debug_dir.join(format!("{}-c.svg", base_name));
        let rust_file = debug_dir.join(format!("{}-rust.svg", base_name));

        if let Err(e) = fs::write(&c_file, c_svg) {
            eprintln!("Warning: Failed to write {}: {}", c_file.display(), e);
        }
        if let Err(e) = fs::write(&rust_file, rust_svg) {
            eprintln!("Warning: Failed to write {}: {}", rust_file.display(), e);
        }

        results.push((
            name_str,
            source,
            c_output,
            rust_output,
            rust_is_err,
            compare_result,
        ));
    }

    // Calculate statistics
    let total = results.len();
    let passed = results.iter().filter(|r| r.5.is_match()).count();
    let pass_rate = if total > 0 {
        (passed as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let mut html = String::new();
    html.push_str(&format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Pikchr C vs Rust Comparison</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Public+Sans:wght@400;500;600;700&display=swap" rel="stylesheet">
    <style>
        html {{
            color-scheme: light dark;
        }}
        * {{
            box-sizing: border-box;
        }}
        body {{
            font-family: 'Public Sans', system-ui, sans-serif;
            margin: 0;
            padding: 0;
            background: light-dark(#eee, #1a1a1a);
            color: light-dark(#333, #e0e0e0);
        }}
        .page {{
            max-width: 1200px;
            margin: 0 auto;
            padding: 24px;
            padding-right: 200px;
        }}
        h1 {{
            font-weight: 600;
            font-size: 20px;
            color: light-dark(#1a1a1a, #e0e0e0);
            margin: 0 0 24px 0;
        }}

        /* Test card */
        .test-card {{
            background: light-dark(white, #2a2a2a);
            border-radius: 8px;
            box-shadow: 0 1px 3px light-dark(rgba(0,0,0,0.08), rgba(0,0,0,0.3));
            margin-bottom: 16px;
            overflow: hidden;
        }}
        .test-header {{
            display: flex;
            align-items: center;
            justify-content: space-between;
            padding: 12px 16px;
            border-bottom: 1px solid light-dark(#eee, #444);
            background: light-dark(#fafafa, #333);
        }}
        .test-title {{
            font-weight: 600;
            font-size: 13px;
            color: light-dark(#333, #e0e0e0);
        }}
        .test-status {{
            font-size: 11px;
            font-weight: 600;
            padding: 3px 8px;
            border-radius: 4px;
        }}
        .test-status.match {{
            background: #dcfce7;
            color: #166534;
        }}
        .test-status.mismatch {{
            background: #fee2e2;
            color: #991b1b;
        }}
        .test-body {{
            padding: 12px 16px;
        }}
        .comparison {{
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 12px;
        }}
        .column {{
            border: 1px solid light-dark(#e5e5e5, #444);
            border-radius: 6px;
            overflow: hidden;
        }}
        .column-header {{
            padding: 8px 12px;
            font-size: 11px;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.3px;
            border-bottom: 1px solid light-dark(#e5e5e5, #444);
        }}
        .c-output .column-header {{
            background: light-dark(#eff6ff, #1e3a8a);
            color: light-dark(#1d4ed8, #93c5fd);
        }}
        .rust-output .column-header {{
            background: light-dark(#fff7ed, #7c2d12);
            color: light-dark(#c2410c, #fdba74);
        }}
        .svg-container {{
            padding: 12px;
            background: light-dark(#fafafa, #333);
            min-height: 60px;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        .svg-container svg {{
            max-width: 100%;
            height: auto;
            max-height: 300px;
        }}
        @media (prefers-color-scheme: dark) {{
            .svg-container svg {{
                filter: invert(1);
            }}
        }}
        .print-output {{
            background: light-dark(#f0f4ff, #1e3a8a);
            border-bottom: 1px solid light-dark(#c0d0f0, #3730a3);
            padding: 8px 12px;
            font-family: 'SF Mono', Monaco, monospace;
            font-size: 11px;
            white-space: pre-wrap;
            color: light-dark(#336, #93c5fd);
        }}
        .error {{
            color: light-dark(#991b1b, #fca5a5);
            font-family: 'SF Mono', Monaco, monospace;
            font-size: 11px;
            white-space: pre-wrap;
            background: light-dark(#fef2f2, #7f1d1d);
            padding: 12px;
        }}
        details {{
            margin-top: 8px;
        }}
        summary {{
            cursor: pointer;
            font-size: 11px;
            font-weight: 500;
            color: light-dark(#666, #999);
            padding: 4px 0;
        }}
        summary:hover {{
            color: light-dark(#333, #e0e0e0);
        }}
        .source {{
            background: light-dark(#f8f8f8, #2a2a2a);
            border: 1px solid light-dark(#e0e0e0, #444);
            padding: 8px 10px;
            font-family: 'SF Mono', Monaco, monospace;
            font-size: 11px;
            white-space: pre-wrap;
            max-height: 150px;
            overflow: auto;
            margin-top: 6px;
            border-radius: 4px;
        }}

        /* Fixed stats badge */
        .stats-badge {{
            position: fixed;
            top: 16px;
            right: 216px;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 10px 16px;
            border-radius: 8px;
            text-align: center;
            box-shadow: 0 4px 12px rgba(102, 126, 234, 0.4);
            z-index: 100;
        }}
        .stats-badge-main {{
            font-size: 20px;
            font-weight: 700;
            line-height: 1;
        }}
        .stats-badge-detail {{
            font-size: 10px;
            margin-top: 4px;
            opacity: 0.9;
        }}

        /* Sidebar */
        nav {{
            position: fixed;
            top: 0;
            right: 0;
            width: 180px;
            height: 100vh;
            background: light-dark(white, #2a2a2a);
            border-left: 1px solid light-dark(#ddd, #444);
            font-size: 11px;
            box-shadow: -2px 0 8px light-dark(rgba(0,0,0,0.05), rgba(0,0,0,0.3));
            display: flex;
            flex-direction: column;
        }}
        .nav-header {{
            padding: 12px;
            border-bottom: 1px solid light-dark(#eee, #444);
            font-weight: 600;
            font-size: 11px;
            text-transform: uppercase;
            letter-spacing: 0.5px;
            color: light-dark(#666, #999);
            flex-shrink: 0;
        }}
        .nav-list {{
            flex: 1;
            overflow-y: auto;
            padding: 8px;
        }}
        nav a {{
            display: flex;
            align-items: center;
            padding: 5px 8px;
            text-decoration: none;
            color: light-dark(#333, #e0e0e0);
            border-radius: 4px;
            margin: 1px 0;
            transition: background 0.15s ease;
        }}
        nav a:hover {{
            background: light-dark(#f0f0f0, #444);
        }}
        .status-dot {{
            width: 6px;
            height: 6px;
            border-radius: 50%;
            margin-right: 6px;
            flex-shrink: 0;
        }}
        .status-pass .status-dot {{
            background: #22863a;
        }}
        .status-fail .status-dot {{
            background: #cb2431;
        }}
        .test-name {{
            overflow: hidden;
            text-overflow: ellipsis;
            white-space: nowrap;
            font-size: 11px;
        }}
    </style>
</head>
<body>

<div class="stats-badge">
    <div class="stats-badge-main">{:.0}%</div>
    <div class="stats-badge-detail">{} / {} passing</div>
</div>

<nav>
    <div class="nav-header">Tests</div>
    <div class="nav-list">
"#,
        pass_rate, passed, total
    ));

    // Navigation links
    for (name_str, _, _, _, _, compare_result) in &results {
        let status_class = if compare_result.is_match() {
            "status-pass"
        } else {
            "status-fail"
        };
        html.push_str(&format!(
            "    <a href=\"#{}\" class=\"{}\"><span class=\"status-dot\"></span><span class=\"test-name\">{}</span></a>\n",
            name_str, status_class, name_str
        ));
    }
    html.push_str(
        "    </div>\n</nav>\n\n<div class=\"page\">\n<h1>Pikchr C vs Rust Comparison</h1>\n",
    );

    // Test content
    for (name_str, source, c_output, rust_output, rust_is_err, compare_result) in &results {
        let status_class = if compare_result.is_match() {
            "match"
        } else {
            "mismatch"
        };
        let status_text = if compare_result.is_match() {
            "MATCH"
        } else {
            "MISMATCH"
        };

        // Check if C output is an error
        let c_is_error = c_output.contains("ERROR:");

        // Extract print output and SVG separately
        let c_print = extract_pre_svg_text(c_output);
        let c_svg = extract_svg(c_output);
        let rust_print = extract_pre_svg_text(rust_output);
        let rust_svg = extract_svg(rust_output);

        // Build C output HTML
        let c_content = if c_is_error {
            format!(r#"<div class="error">{}</div>"#, html_escape(c_output))
        } else {
            let mut content = String::new();
            if let Some(print_text) = c_print {
                content.push_str(&format!(
                    r#"<div class="print-output">{}</div>"#,
                    html_escape(print_text)
                ));
            }
            if let Some(svg) = c_svg {
                content.push_str(svg);
            } else if c_output.contains("<!--") {
                // Non-SVG output like empty diagram comment
                content.push_str(&format!(
                    r#"<div class="print-output">{}</div>"#,
                    html_escape(c_output.trim())
                ));
            }
            if content.is_empty() {
                c_output.clone()
            } else {
                content
            }
        };

        // Build Rust output HTML
        let rust_content = if *rust_is_err {
            format!(r#"<div class="error">{}</div>"#, html_escape(rust_output))
        } else {
            let mut content = String::new();
            if let Some(print_text) = rust_print {
                content.push_str(&format!(
                    r#"<div class="print-output">{}</div>"#,
                    html_escape(print_text)
                ));
            }
            if let Some(svg) = rust_svg {
                content.push_str(svg);
            } else if rust_output.contains("<!--") {
                // Non-SVG output like empty diagram comment
                content.push_str(&format!(
                    r#"<div class="print-output">{}</div>"#,
                    html_escape(rust_output.trim())
                ));
            }
            if content.is_empty() {
                rust_output.clone()
            } else {
                content
            }
        };

        html.push_str(&format!(
            r#"
<div class="test-card" id="{}">
    <div class="test-header">
        <span class="test-title">{}</span>
        <span class="test-status {}">{}</span>
    </div>
    <div class="test-body">
        <div class="comparison">
            <div class="column c-output">
                <div class="column-header">C pikchr</div>
                <div class="svg-container">{}</div>
            </div>
            <div class="column rust-output">
                <div class="column-header">Rust pikchr</div>
                <div class="svg-container">{}</div>
            </div>
        </div>
        <details>
            <summary>Source</summary>
            <div class="source">{}</div>
        </details>
    </div>
</div>
"#,
            name_str,
            name_str,
            status_class,
            status_text,
            c_content,
            rust_content,
            html_escape(source),
        ));
    }

    html.push_str("</div>\n</body></html>");

    fs::write(&output_path, html).expect("Failed to write HTML");
    eprintln!("Generated comparison at: {}", output_path.display());
    eprintln!("Individual SVG files written to: {}", debug_dir.display());
}

fn run_c_pikchr(c_pikchr: &str, source: &str) -> String {
    use std::io::Write;

    let mut child = Command::new(c_pikchr)
        .arg("--svg-only")
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run C pikchr");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = child
        .wait_with_output()
        .expect("failed to wait on C pikchr");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn generate_pngs() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let debug_svg_dir = Path::new(manifest_dir).join("../debug-svg");

    if !debug_svg_dir.exists() {
        eprintln!(
            "debug-svg directory not found. Run 'cargo test' or 'cargo xtask compare-html' first."
        );
        std::process::exit(1);
    }

    // Check if rsvg-convert is available
    if Command::new("rsvg-convert")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("rsvg-convert not found. Please install librsvg2-bin (Ubuntu/Debian) or librsvg (macOS)");
        std::process::exit(1);
    }

    println!("Generating PNG files from SVG outputs...");

    let entries = fs::read_dir(&debug_svg_dir).expect("Failed to read debug-svg directory");
    let mut svg_files = Vec::new();

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("svg") {
            svg_files.push(path);
        }
    }

    if svg_files.is_empty() {
        eprintln!("No SVG files found in debug-svg directory");
        std::process::exit(1);
    }

    let processed = AtomicUsize::new(0);
    let total = svg_files.len();

    svg_files.par_iter().for_each(|svg_file| {
        let png_file = svg_file.with_extension("png");

        let output = Command::new("rsvg-convert")
            .arg("-w")
            .arg("400") // Set width to 400px for consistent sizing
            .arg("-h")
            .arg("400") // Set height to 400px
            .arg(&svg_file)
            .arg("-o")
            .arg(&png_file)
            .output()
            .expect("Failed to run rsvg-convert");

        let count = processed.fetch_add(1, Ordering::Relaxed) + 1;
        if output.status.success() {
            println!(
                "[{}/{}] Converted {} ✓",
                count,
                total,
                svg_file.file_name().unwrap().to_str().unwrap()
            );
        } else {
            eprintln!(
                "[{}/{}] Failed to convert {}: {}",
                count,
                total,
                svg_file.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    });

    println!(
        "Generated {} PNG files in {}",
        svg_files.len(),
        debug_svg_dir.display()
    );
    println!("You can now view the PNG files to visually compare C vs Rust outputs.");
}
