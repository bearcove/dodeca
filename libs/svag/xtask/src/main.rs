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
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
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

    // Only use top-level SVGs for README benchmarks
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
