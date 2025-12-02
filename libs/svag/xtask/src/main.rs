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
use std::time::Instant;
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
    /// Per-file results: (name, original_size, minified_size, time_ms)
    files: Vec<(String, usize, usize, f64)>,
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
                f["time_ms"].as_f64()?,
            ))
        })
        .collect();

    let total = &json["total"];
    Some(SvgoResults {
        files,
        total_time_ms: total["time_ms"].as_f64()?,
    })
}

fn cmd_readme() {
    let root = project_root();
    let template_path = root.join("README.tmpl.md");
    let output_path = root.join("README.md");
    let corpus_dir = root.join("tests/corpus");

    // Only use top-level SVGs for README benchmarks
    let mut svg_files: Vec<_> = fs::read_dir(&corpus_dir)
        .expect("Failed to read corpus directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .collect();
    svg_files.sort_by_key(|e| e.path());

    println!("Running benchmarks on {} files...\n", svg_files.len());

    // Run svgo batch first (loads Node once, processes all files)
    println!("Running svgo (batch mode)...");
    let svgo_results = run_svgo_batch(&corpus_dir);
    if svgo_results.is_none() {
        eprintln!("Warning: svgo not found. Install with: npm install svgo");
        eprintln!("Continuing without svgo comparison...\n");
    }

    // Build a map of svgo results by filename
    let svgo_by_name: std::collections::HashMap<String, (usize, f64)> = svgo_results
        .as_ref()
        .map(|r| {
            r.files
                .iter()
                .map(|(name, _, minified, time)| (name.clone(), (*minified, *time)))
                .collect()
        })
        .unwrap_or_default();

    // Run svag on all files (batch timing)
    println!("Running svag...");
    let svag_start = Instant::now();

    let mut benchmarks = Vec::new();
    let mut total_original = 0usize;
    let mut total_svag = 0usize;
    let mut total_svgo = 0usize;

    for entry in &svg_files {
        let path = entry.path();
        let file_stem = path.file_stem().unwrap().to_string_lossy();
        let name = file_stem
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

        // Get svgo result from batch
        let svgo_size = svgo_by_name
            .get(file_stem.as_ref())
            .map(|(size, _)| *size)
            .unwrap_or(original_size);
        total_svgo += svgo_size;

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

    let svag_time = svag_start.elapsed();
    let svgo_time_ms = svgo_results.as_ref().map(|r| r.total_time_ms).unwrap_or(0.0);

    let svag_saved = total_original.saturating_sub(total_svag);
    let svgo_saved = total_original.saturating_sub(total_svgo);

    println!("\n--- Totals ---");
    println!(
        "Original: {} | svag: {} ({}, saved {}) | svgo: {} ({}, saved {})",
        format_bytes(total_original),
        format_bytes(total_svag),
        pct_reduction(total_original, total_svag),
        format_bytes(svag_saved),
        format_bytes(total_svgo),
        pct_reduction(total_original, total_svgo),
        format_bytes(svgo_saved),
    );
    println!(
        "Time: svag: {} | svgo: {}",
        format_duration(svag_time.as_secs_f64() * 1000.0),
        format_duration(svgo_time_ms),
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
                svag_saved => format_bytes(svag_saved),
                svag_time => format_duration(svag_time.as_secs_f64() * 1000.0),
                svgo => format_bytes(total_svgo),
                svgo_pct => pct_reduction(total_original, total_svgo),
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
