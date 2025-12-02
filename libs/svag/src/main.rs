use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use ignore::WalkBuilder;
use rayon::prelude::*;
use svag::{Options, minify_with_options};

#[derive(Parser)]
#[command(name = "svag")]
#[command(about = "An SVG minifier", long_about = None)]
struct Cli {
    /// Input file or directory (use - for stdin)
    #[arg(default_value = "-")]
    input: PathBuf,

    /// Output file (use - for stdout). For directory mode, files are minified in-place.
    #[arg(short, long, default_value = "-")]
    output: PathBuf,

    /// Precision for coordinates (decimal places)
    #[arg(short, long, default_value = "2")]
    precision: u8,

    /// Keep XML declaration
    #[arg(long)]
    keep_xml_declaration: bool,

    /// Keep DOCTYPE
    #[arg(long)]
    keep_doctype: bool,

    /// Keep comments
    #[arg(long)]
    keep_comments: bool,

    /// Disable path minification
    #[arg(long)]
    no_minify_paths: bool,

    /// Disable color minification
    #[arg(long)]
    no_minify_colors: bool,

    /// Disable all optimizations (just parse and re-serialize)
    #[arg(long)]
    no_optimize: bool,

    /// Print size comparison
    #[arg(short, long)]
    stats: bool,

    /// Benchmark mode: process files but don't write output, print JSON stats
    #[arg(long)]
    bench: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Build options
    let options = if cli.no_optimize {
        Options {
            remove_comments: false,
            remove_metadata: false,
            remove_xml_declaration: false,
            remove_doctype: false,
            remove_unused_namespaces: false,
            collapse_groups: false,
            remove_hidden: false,
            remove_empty: false,
            minify_colors: false,
            remove_defaults: false,
            minify_paths: false,
            minify_styles: false,
            merge_paths: false,
            sort_attrs: false,
            precision: cli.precision,
        }
    } else {
        Options {
            precision: cli.precision,
            remove_xml_declaration: !cli.keep_xml_declaration,
            remove_doctype: !cli.keep_doctype,
            remove_comments: !cli.keep_comments,
            minify_paths: !cli.no_minify_paths,
            minify_colors: !cli.no_minify_colors,
            ..Options::default()
        }
    };

    // Check if input is a directory
    if cli.input.is_dir() {
        process_directory(&cli, &options)?;
    } else {
        process_single_file(&cli, &options)?;
    }

    Ok(())
}

fn process_single_file(cli: &Cli, options: &Options) -> Result<(), Box<dyn std::error::Error>> {
    // Read input
    let input = if cli.input.as_os_str() == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(&cli.input)?
    };

    let input_len = input.len();

    // Minify
    let output = minify_with_options(&input, options)?;
    let output_len = output.len();

    // Write output
    if cli.output.as_os_str() == "-" {
        io::stdout().write_all(output.as_bytes())?;
    } else {
        fs::write(&cli.output, &output)?;
    }

    // Print stats if requested
    if cli.stats {
        let saved = input_len.saturating_sub(output_len);
        let percent = if input_len > 0 {
            (saved as f64 / input_len as f64) * 100.0
        } else {
            0.0
        };
        eprintln!(
            "{} -> {} bytes ({:.1}% smaller)",
            input_len, output_len, percent
        );
    }

    Ok(())
}

fn process_directory(cli: &Cli, options: &Options) -> Result<(), Box<dyn std::error::Error>> {
    // Collect all SVG files
    let files: Vec<PathBuf> = WalkBuilder::new(&cli.input)
        .git_ignore(false)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "svg"))
        .map(|e| e.path().to_path_buf())
        .collect();

    let file_count = files.len();

    if cli.bench {
        // Benchmark mode: process in parallel, collect stats
        let total_original = AtomicUsize::new(0);
        let total_minified = AtomicUsize::new(0);
        let success_count = AtomicUsize::new(0);
        let fail_count = AtomicUsize::new(0);

        let start = std::time::Instant::now();

        files.par_iter().for_each(|path| {
            if let Ok(input) = fs::read_to_string(path) {
                let input_len = input.len();
                total_original.fetch_add(input_len, Ordering::Relaxed);

                match minify_with_options(&input, options) {
                    Ok(output) => {
                        total_minified.fetch_add(output.len(), Ordering::Relaxed);
                        success_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        total_minified.fetch_add(input_len, Ordering::Relaxed);
                        fail_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        let elapsed = start.elapsed();
        let orig = total_original.load(Ordering::Relaxed);
        let mini = total_minified.load(Ordering::Relaxed);
        let succ = success_count.load(Ordering::Relaxed);
        let fail = fail_count.load(Ordering::Relaxed);

        // Output JSON for easy parsing
        println!(
            r#"{{"files":{},"success":{},"failed":{},"original":{},"minified":{},"saved":{},"time_ms":{:.2}}}"#,
            file_count,
            succ,
            fail,
            orig,
            mini,
            orig.saturating_sub(mini),
            elapsed.as_secs_f64() * 1000.0
        );
    } else {
        // Regular mode: minify in-place
        let processed = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);

        files.par_iter().for_each(|path| {
            if let Ok(input) = fs::read_to_string(path) {
                match minify_with_options(&input, options) {
                    Ok(output) => {
                        if fs::write(path, &output).is_ok() {
                            processed.fetch_add(1, Ordering::Relaxed);
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(_) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });

        if cli.stats {
            eprintln!(
                "Processed {} files, {} failed",
                processed.load(Ordering::Relaxed),
                failed.load(Ordering::Relaxed)
            );
        }
    }

    Ok(())
}
