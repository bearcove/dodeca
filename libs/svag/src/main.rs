use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use clap::Parser;
use savage::{minify_with_options, Options};

#[derive(Parser)]
#[command(name = "savage")]
#[command(about = "A savage SVG minifier", long_about = None)]
struct Cli {
    /// Input file (use - for stdin)
    #[arg(default_value = "-")]
    input: PathBuf,

    /// Output file (use - for stdout)
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Read input
    let input = if cli.input.as_os_str() == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        fs::read_to_string(&cli.input)?
    };

    let input_len = input.len();

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

    // Minify
    let output = minify_with_options(&input, &options)?;
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
