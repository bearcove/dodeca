//! Generates README.md from the template with fresh benchmark data.
//!
//! Compares savage against svgo for both size reduction and speed.
//!
//! Usage: cargo run --bin generate-readme

use minijinja::{context, Environment};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const TEMPLATE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/README.tmpl.md");
const OUTPUT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");
const CORPUS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");

/// Format bytes in a human-readable way
fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Format duration in a human-readable way
fn format_duration(d: Duration) -> String {
    let micros = d.as_micros();
    if micros >= 1_000_000 {
        format!("{:.2}s", d.as_secs_f64())
    } else if micros >= 1_000 {
        format!("{:.2}ms", micros as f64 / 1000.0)
    } else {
        format!("{}µs", micros)
    }
}

/// Format throughput
fn format_throughput(bytes: usize, duration: Duration) -> String {
    let bytes_per_sec = bytes as f64 / duration.as_secs_f64();
    if bytes_per_sec >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bytes_per_sec / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B/s", bytes_per_sec)
    }
}

/// Calculate percentage reduction
fn pct_reduction(original: usize, minified: usize) -> String {
    let pct = (1.0 - minified as f64 / original as f64) * 100.0;
    format!("-{:.1}%", pct)
}

/// Run svgo on a file and return the minified content
fn run_svgo(svgo_cmd: &str, input: &str) -> Option<String> {
    let mut child = Command::new(svgo_cmd)
        .args(["--input", "-", "--output", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    use std::io::Write;
    // Take ownership of stdin so it gets dropped after write
    let mut stdin = child.stdin.take()?;
    stdin.write_all(input.as_bytes()).ok()?;
    drop(stdin); // Close stdin to signal EOF

    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

/// Find svgo command - checks local node_modules first, then global
fn find_svgo() -> Option<String> {
    // Check local node_modules first
    let local = concat!(env!("CARGO_MANIFEST_DIR"), "/node_modules/.bin/svgo");
    if std::path::Path::new(local).exists() {
        return Some(local.to_string());
    }
    // Check global
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

fn main() {
    // Check if svgo is available
    let svgo_cmd = find_svgo();

    if svgo_cmd.is_none() {
        eprintln!("Warning: svgo not found. Install with: npm install svgo");
        eprintln!("Continuing without svgo comparison...\n");
    }

    // Collect all SVG files
    let corpus_path = Path::new(CORPUS_DIR);
    let mut svg_files: Vec<_> = fs::read_dir(corpus_path)
        .expect("Failed to read corpus directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .collect();
    svg_files.sort_by_key(|e| e.path());

    let mut benchmarks = Vec::new();
    let mut total_original = 0usize;
    let mut total_savage = 0usize;
    let mut total_svgo = 0usize;
    let mut total_savage_time = Duration::ZERO;
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

        // Benchmark savage
        let start = Instant::now();
        let savage_result = savage::minify(&svg).expect("savage failed");
        let savage_time = start.elapsed();
        let savage_size = savage_result.len();
        total_savage += savage_size;
        total_savage_time += savage_time;

        // Benchmark svgo
        let (svgo_size, svgo_time) = if let Some(ref cmd) = svgo_cmd {
            let start = Instant::now();
            if let Some(svgo_result) = run_svgo(cmd, &svg) {
                let elapsed = start.elapsed();
                (svgo_result.len(), elapsed)
            } else {
                (original_size, Duration::ZERO)
            }
        } else {
            (original_size, Duration::ZERO)
        };
        total_svgo += svgo_size;
        total_svgo_time += svgo_time;

        println!(
            "{}: {} → savage: {} ({}), svgo: {} ({})",
            name,
            format_bytes(original_size),
            format_bytes(savage_size),
            pct_reduction(original_size, savage_size),
            format_bytes(svgo_size),
            pct_reduction(original_size, svgo_size),
        );

        benchmarks.push(context! {
            name => name,
            original => format_bytes(original_size),
            savage => format_bytes(savage_size),
            savage_pct => pct_reduction(original_size, savage_size),
            svgo => format_bytes(svgo_size),
            svgo_pct => pct_reduction(original_size, svgo_size),
        });
    }

    let speedup = if total_svgo_time > Duration::ZERO {
        format!("{:.0}x", total_svgo_time.as_secs_f64() / total_savage_time.as_secs_f64())
    } else {
        "N/A".to_string()
    };

    println!("\n--- Totals ---");
    println!(
        "Original: {} | savage: {} ({}) | svgo: {} ({})",
        format_bytes(total_original),
        format_bytes(total_savage),
        pct_reduction(total_original, total_savage),
        format_bytes(total_svgo),
        pct_reduction(total_original, total_svgo),
    );
    println!(
        "Time: savage {} | svgo {} | speedup: {}",
        format_duration(total_savage_time),
        format_duration(total_svgo_time),
        speedup
    );

    // Render template
    let template = fs::read_to_string(TEMPLATE_PATH).expect("Failed to read template");
    let mut env = Environment::new();
    env.add_template("readme", &template).unwrap();

    let tmpl = env.get_template("readme").unwrap();
    let rendered = tmpl
        .render(context! {
            file_count => svg_files.len(),
            benchmarks => benchmarks,
            total => context! {
                original => format_bytes(total_original),
                savage => format_bytes(total_savage),
                savage_pct => pct_reduction(total_original, total_savage),
                svgo => format_bytes(total_svgo),
                svgo_pct => pct_reduction(total_original, total_svgo),
            },
            timing => context! {
                savage_time => format_duration(total_savage_time),
                savage_throughput => format_throughput(total_original, total_savage_time),
                svgo_time => format_duration(total_svgo_time),
                svgo_throughput => if total_svgo_time > Duration::ZERO {
                    format_throughput(total_original, total_svgo_time)
                } else {
                    "N/A".to_string()
                },
                speedup => speedup,
            },
        })
        .expect("Failed to render template");

    fs::write(OUTPUT_PATH, rendered).expect("Failed to write README.md");
    println!("\nGenerated README.md");
}
