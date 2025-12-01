//! xtask for svag development tasks.
//!
//! Usage: cargo xtask readme

use minijinja::{Environment, context};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn pct_reduction(original: usize, minified: usize) -> String {
    let pct = (1.0 - minified as f64 / original as f64) * 100.0;
    format!("-{:.1}%", pct)
}

fn run_svgo(svgo_cmd: &str, input: &str) -> Option<String> {
    let mut child = Command::new(svgo_cmd)
        .args(["--input", "-", "--output", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    use std::io::Write;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(input.as_bytes()).ok()?;
    drop(stdin);

    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

fn find_svgo() -> Option<String> {
    let local = project_root().join("node_modules/.bin/svgo");
    if local.exists() {
        return Some(local.to_string_lossy().into_owned());
    }
    if Command::new("svgo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Some("svgo".to_string());
    }
    None
}

fn cmd_readme() {
    let root = project_root();
    let template_path = root.join("README.tmpl.md");
    let output_path = root.join("README.md");
    let corpus_dir = root.join("tests/corpus");

    let svgo_cmd = find_svgo();
    if svgo_cmd.is_none() {
        eprintln!("Warning: svgo not found. Install with: npm install svgo");
        eprintln!("Continuing without svgo comparison...\n");
    }

    let mut svg_files: Vec<_> = fs::read_dir(&corpus_dir)
        .expect("Failed to read corpus directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .collect();
    svg_files.sort_by_key(|e| e.path());

    let mut benchmarks = Vec::new();
    let mut total_original = 0usize;
    let mut total_svag = 0usize;
    let mut total_svgo = 0usize;
    let mut total_svgo_time = Duration::ZERO;

    println!("Running benchmarks on {} files...\n", svg_files.len());

    for entry in &svg_files {
        let path = entry.path();
        let name = path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .replace('-', " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let svg = fs::read_to_string(&path).expect("Failed to read SVG");
        let original_size = svg.len();
        total_original += original_size;

        // Run svag
        let svag_result = svag::minify(&svg).expect("svag failed");
        let svag_size = svag_result.len();
        total_svag += svag_size;

        // Run svgo
        let (svgo_size, svgo_time) = if let Some(ref cmd) = svgo_cmd {
            let start = Instant::now();
            if let Some(svgo_result) = run_svgo(cmd, &svg) {
                (svgo_result.len(), start.elapsed())
            } else {
                (original_size, Duration::ZERO)
            }
        } else {
            (original_size, Duration::ZERO)
        };
        total_svgo += svgo_size;
        total_svgo_time += svgo_time;

        println!(
            "{}: {} → svag: {} ({}), svgo: {} ({})",
            name,
            format_bytes(original_size),
            format_bytes(svag_size),
            pct_reduction(original_size, svag_size),
            format_bytes(svgo_size),
            pct_reduction(original_size, svgo_size),
        );

        benchmarks.push(context! {
            name => name,
            original => format_bytes(original_size),
            svag => format_bytes(svag_size),
            svag_pct => pct_reduction(original_size, svag_size),
            svgo => format_bytes(svgo_size),
            svgo_pct => pct_reduction(original_size, svgo_size),
        });
    }

    println!("\n--- Totals ---");
    println!(
        "Original: {} | svag: {} ({}) | svgo: {} ({})",
        format_bytes(total_original),
        format_bytes(total_svag),
        pct_reduction(total_original, total_svag),
        format_bytes(total_svgo),
        pct_reduction(total_original, total_svgo),
    );

    // Render template
    let template = fs::read_to_string(&template_path).expect("Failed to read template");
    let mut env = Environment::new();
    env.add_template("readme", &template).unwrap();

    let tmpl = env.get_template("readme").unwrap();
    let rendered = tmpl
        .render(context! {
            file_count => svg_files.len(),
            benchmarks => benchmarks,
            total => context! {
                original => format_bytes(total_original),
                svag => format_bytes(total_svag),
                svag_pct => pct_reduction(total_original, total_svag),
                svgo => format_bytes(total_svgo),
                svgo_pct => pct_reduction(total_original, total_svgo),
            },
        })
        .expect("Failed to render template");

    fs::write(&output_path, rendered).expect("Failed to write README.md");
    println!("\nGenerated README.md");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("readme") => cmd_readme(),
        Some(cmd) => {
            eprintln!("Unknown command: {}", cmd);
            eprintln!("Available commands: readme");
            std::process::exit(1);
        }
        None => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("Available commands: readme");
            std::process::exit(1);
        }
    }
}
