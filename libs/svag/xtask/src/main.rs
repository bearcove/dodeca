//! xtask for svag development tasks.
//!
//! Usage:
//!   cargo xtask readme        - Generate README.md with benchmarks
//!   cargo xtask fetch-corpus  - Download SVG test corpus

use flate2::read::GzDecoder;
use ignore::WalkBuilder;
use minijinja::{Environment, context};
use rapidhash::RapidHashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use tar::Archive;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn corpus_dir() -> PathBuf {
    project_root().join("tests/corpus")
}

// ============================================================================
// fetch-corpus command
// ============================================================================

const W3C_URL: &str =
    "https://www.w3.org/Graphics/SVG/Test/20110816/archives/W3C_SVG_11_TestSuite.tar.gz";
const OXYGEN_VERSION: &str = "5.116";
const WIKIMEDIA_SVGS: &[&str] = &[
    "https://upload.wikimedia.org/wikipedia/commons/a/a1/Spain_languages-de.svg",
    "https://upload.wikimedia.org/wikipedia/commons/d/d1/Saariston_Rengastie_route_labels.svg",
    "https://upload.wikimedia.org/wikipedia/commons/5/5a/Mapa_do_Brasil_por_c%C3%B3digo_DDD.svg",
    "https://upload.wikimedia.org/wikipedia/commons/c/c1/Propane_flame_contours-en.svg",
    "https://upload.wikimedia.org/wikipedia/commons/f/ff/1_42_polytope_7-cube.svg",
    "https://upload.wikimedia.org/wikipedia/commons/f/fd/Germany_%28%2Bdistricts_%2Bmunicipalities%29_location_map_current.svg",
    "https://upload.wikimedia.org/wikipedia/commons/7/7f/Italy_-_Regions_and_provinces.svg",
    "https://upload.wikimedia.org/wikipedia/commons/6/60/Aegean_sea_Anatolia_and_Armenian_highlands_regions_large_topographic_basemap.svg",
];

fn download(url: &str) -> reqwest::Result<Vec<u8>> {
    println!("  Downloading {}...", url);
    let resp = reqwest::blocking::get(url)?;
    resp.bytes().map(|b| b.to_vec())
}

fn normalize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '/' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn fetch_w3c_test_suite(dest: &Path) -> io::Result<usize> {
    println!("Fetching W3C SVG 1.1 Test Suite...");
    let data = download(W3C_URL).expect("Failed to download W3C test suite");

    let decoder = GzDecoder::new(&data[..]);
    let mut archive = Archive::new(decoder);

    let w3c_dir = dest.join("w3c");
    fs::create_dir_all(&w3c_dir)?;

    let mut count = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let path_str = path.to_string_lossy();

        // Only extract .svg files from the svg/ directory
        if path_str.starts_with("svg/") && path_str.ends_with(".svg") {
            let filename = path.file_name().unwrap().to_string_lossy();
            let normalized = normalize_filename(&filename);
            let dest_path = w3c_dir.join(&normalized);

            let mut contents = Vec::new();
            entry.read_to_end(&mut contents)?;
            fs::write(&dest_path, &contents)?;
            count += 1;
        }
    }

    println!("  Extracted {} SVG files", count);
    Ok(count)
}

fn fetch_oxygen_icons(dest: &Path) -> io::Result<usize> {
    println!("Fetching KDE Oxygen Icons...");
    let url = format!(
        "https://download.kde.org/stable/frameworks/{}/oxygen-icons-{}.0.tar.xz",
        OXYGEN_VERSION, OXYGEN_VERSION
    );

    let oxygen_dir = dest.join("oxygen");
    fs::create_dir_all(&oxygen_dir)?;

    // Extract both .svg and .svgz files
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "curl -sL '{}' | xz -d | tar -xf - --strip-components=1 -C '{}' --wildcards '*.svg' '*.svgz'",
            url,
            oxygen_dir.display()
        ))
        .output()?;

    if !output.status.success() {
        eprintln!("  Warning: Failed to extract Oxygen icons");
        eprintln!("  stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Ok(0);
    }

    // Decompress .svgz files (they're gzip-compressed SVGs)
    let svgz_files: Vec<_> = WalkBuilder::new(&oxygen_dir)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svgz"))
        .map(|e| e.path().to_path_buf())
        .collect();

    for svgz_path in &svgz_files {
        let svg_path = svgz_path.with_extension("svg");
        if let Ok(compressed) = fs::read(svgz_path) {
            let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
            let mut decompressed = Vec::new();
            if decoder.read_to_end(&mut decompressed).is_ok() {
                let _ = fs::write(&svg_path, &decompressed);
            }
        }
        let _ = fs::remove_file(svgz_path);
    }

    // Count extracted files
    let count = WalkBuilder::new(&oxygen_dir)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .count();

    println!("  Extracted {} SVG files", count);
    Ok(count)
}

fn fetch_wikimedia_commons(dest: &Path) -> io::Result<usize> {
    println!("Fetching Wikimedia Commons SVGs...");

    let wikimedia_dir = dest.join("wikimedia");
    fs::create_dir_all(&wikimedia_dir)?;

    let mut count = 0;
    for url in WIKIMEDIA_SVGS {
        let filename = url.rsplit('/').next().unwrap();
        let normalized = normalize_filename(filename);
        let dest_path = wikimedia_dir.join(&normalized);

        if dest_path.exists() {
            println!("  Skipping {} (already exists)", normalized);
            continue;
        }

        match download(url) {
            Ok(data) => {
                fs::write(&dest_path, &data)?;
                count += 1;
            }
            Err(e) => {
                eprintln!("  Warning: Failed to download {}: {}", url, e);
            }
        }
    }

    println!("  Downloaded {} SVG files", count);
    Ok(count)
}

fn deduplicate(dest: &Path) -> io::Result<usize> {
    println!("Deduplicating...");

    let mut seen: RapidHashMap<u64, PathBuf> = RapidHashMap::default();
    let mut removed = 0;

    for entry in WalkBuilder::new(dest).build() {
        let entry = entry.map_err(io::Error::other)?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|e| e == "svg") {
            let contents = fs::read(path)?;
            let hash = rapidhash::rapidhash(&contents);

            if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(hash) {
                e.insert(path.to_path_buf());
            } else {
                fs::remove_file(path)?;
                removed += 1;
            }
        }
    }

    println!("  Removed {} duplicates", removed);
    Ok(removed)
}

fn count_svgs(dest: &Path) -> usize {
    WalkBuilder::new(dest)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .count()
}

fn cmd_fetch_corpus() {
    let dest = corpus_dir();

    println!("Fetching SVG test corpus to {:?}\n", dest);

    // Create corpus directory
    fs::create_dir_all(&dest).expect("Failed to create corpus directory");

    // Fetch from all sources
    fetch_w3c_test_suite(&dest).expect("Failed to fetch W3C test suite");
    fetch_oxygen_icons(&dest).expect("Failed to fetch Oxygen icons");
    fetch_wikimedia_commons(&dest).expect("Failed to fetch Wikimedia Commons");

    // Deduplicate
    deduplicate(&dest).expect("Failed to deduplicate");

    // Count total
    let total = count_svgs(&dest);
    println!("\nTotal: {} SVG files in corpus", total);
}

// ============================================================================
// readme command
// ============================================================================

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_duration(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else {
        format!("{:.1}ms", ms)
    }
}

fn pct_reduction(original: usize, minified: usize) -> String {
    let pct = (1.0 - minified as f64 / original as f64) * 100.0;
    format!("-{:.1}%", pct)
}

/// Result from the batch svgo benchmark script
#[derive(Debug)]
struct SvgoResults {
    /// Per-file results: (name, original_size, minified_size)
    files: Vec<(String, usize, usize)>,
    total_time_ms: f64,
}

fn run_svgo_batch(corpus_dir: &Path) -> Option<SvgoResults> {
    let script = project_root().join("bench-svgo.mjs");
    if !script.exists() {
        return None;
    }

    let output = Command::new("node")
        .arg(&script)
        .arg(corpus_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        eprintln!("svgo batch failed: {}", String::from_utf8_lossy(&output.stderr));
        return None;
    }

    // Parse JSON output
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;

    let files = json["files"]
        .as_array()?
        .iter()
        .filter_map(|f| {
            Some((
                f["name"].as_str()?.to_string(),
                f["original"].as_u64()? as usize,
                f["minified"].as_u64()? as usize,
            ))
        })
        .collect();

    let total = &json["total"];
    Some(SvgoResults {
        files,
        total_time_ms: total["time_ms"].as_f64()?,
    })
}

/// Result from svag --bench
#[derive(Debug)]
struct SvagBenchResult {
    files: usize,
    success: usize,
    failed: usize,
    original: usize,
    minified: usize,
    time_ms: f64,
}

fn run_svag_bench(corpus_dir: &Path) -> Option<SvagBenchResult> {
    // Build svag in release mode first
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "svag"])
        .current_dir(project_root())
        .status()
        .ok()?;

    if !status.success() {
        eprintln!("Failed to build svag");
        return None;
    }

    let svag_bin = project_root().join("target/release/svag");
    let output = Command::new(&svag_bin)
        .arg(corpus_dir)
        .arg("--bench")
        .output()
        .ok()?;

    if !output.status.success() {
        eprintln!("svag bench failed: {}", String::from_utf8_lossy(&output.stderr));
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;

    Some(SvagBenchResult {
        files: json["files"].as_u64()? as usize,
        success: json["success"].as_u64()? as usize,
        failed: json["failed"].as_u64()? as usize,
        original: json["original"].as_u64()? as usize,
        minified: json["minified"].as_u64()? as usize,
        time_ms: json["time_ms"].as_f64()?,
    })
}

fn cmd_readme() {
    let root = project_root();
    let template_path = root.join("README.tmpl.md");
    let output_path = root.join("README.md");
    let corpus_dir = root.join("tests/corpus");

    println!("Running parallel benchmarks...\n");

    // Run svgo batch (parallel with worker threads)
    println!("Running svgo (parallel worker threads)...");
    let svgo_results = run_svgo_batch(&corpus_dir);
    if svgo_results.is_none() {
        eprintln!("Warning: svgo not found. Install with: npm install svgo");
        eprintln!("Continuing without svgo comparison...\n");
    }

    // Run svag bench (parallel with rayon)
    println!("Running svag (parallel with rayon)...");
    let svag_results = run_svag_bench(&corpus_dir);
    if svag_results.is_none() {
        eprintln!("Error: svag bench failed");
        return;
    }

    let svag = svag_results.unwrap();
    let svgo_time_ms = svgo_results.as_ref().map(|r| r.total_time_ms).unwrap_or(0.0);

    // Calculate svgo totals from file results
    let (svgo_original, svgo_minified) = svgo_results
        .as_ref()
        .map(|r| {
            r.files.iter().fold((0usize, 0usize), |(orig, mini), (_, o, m)| {
                (orig + o, mini + m)
            })
        })
        .unwrap_or((svag.original, svag.original));

    let svag_saved = svag.original.saturating_sub(svag.minified);
    let svgo_saved = svgo_original.saturating_sub(svgo_minified);

    println!("\n--- Results ---");
    println!("Files: {} (svag: {} success, {} failed)", svag.files, svag.success, svag.failed);
    println!(
        "Original: {} | svag: {} ({}, saved {}) | svgo: {} ({}, saved {})",
        format_bytes(svag.original),
        format_bytes(svag.minified),
        pct_reduction(svag.original, svag.minified),
        format_bytes(svag_saved),
        format_bytes(svgo_minified),
        pct_reduction(svgo_original, svgo_minified),
        format_bytes(svgo_saved),
    );
    println!(
        "Time: svag: {} | svgo: {}",
        format_duration(svag.time_ms),
        format_duration(svgo_time_ms),
    );

    // Render template
    let template = fs::read_to_string(&template_path).expect("Failed to read template");
    let mut env = Environment::new();
    env.add_template("readme", &template).unwrap();

    let tmpl = env.get_template("readme").unwrap();
    let rendered = tmpl
        .render(context! {
            file_count => svag.files,
            total => context! {
                original => format_bytes(svag.original),
                svag => format_bytes(svag.minified),
                svag_pct => pct_reduction(svag.original, svag.minified),
                svag_saved => format_bytes(svag_saved),
                svag_time => format_duration(svag.time_ms),
                svgo => format_bytes(svgo_minified),
                svgo_pct => pct_reduction(svgo_original, svgo_minified),
                svgo_saved => format_bytes(svgo_saved),
                svgo_time => format_duration(svgo_time_ms),
            },
        })
        .expect("Failed to render template");

    fs::write(&output_path, rendered).expect("Failed to write README.md");
    println!("\nGenerated README.md");
}

// ============================================================================
// main
// ============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("readme") => cmd_readme(),
        Some("fetch-corpus") => cmd_fetch_corpus(),
        Some(cmd) => {
            eprintln!("Unknown command: {}", cmd);
            eprintln!("Available commands: readme, fetch-corpus");
            std::process::exit(1);
        }
        None => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("Available commands: readme, fetch-corpus");
            std::process::exit(1);
        }
    }
}
