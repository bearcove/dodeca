use std::fs;
use std::io::{self, Read, Write};

use aasvg::RenderOptions;
use facet::Facet;
use facet_args as args;

/// Convert ASCII art diagrams to SVG
#[derive(Facet, Debug)]
struct Args {
    /// Input file (reads from stdin if not provided)
    #[facet(default, args::positional)]
    input: Option<String>,

    /// Output file (writes to stdout if not provided)
    #[facet(default, args::named, args::short = 'o')]
    output: Option<String>,

    /// Add a backdrop rectangle for dark mode compatibility
    #[facet(args::named)]
    backdrop: bool,
}

fn main() {
    let args: Args = match args::from_std_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let input = match &args.input {
        Some(path) => fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Failed to read {}: {}", path, e);
            std::process::exit(1);
        }),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
                eprintln!("Failed to read stdin: {}", e);
                std::process::exit(1);
            });
            buf
        }
    };

    let options = RenderOptions::new().with_backdrop(args.backdrop);
    let svg = aasvg::render_with_options(&input, &options);

    match &args.output {
        Some(path) => {
            fs::write(path, &svg).unwrap_or_else(|e| {
                eprintln!("Failed to write {}: {}", path, e);
                std::process::exit(1);
            });
        }
        None => {
            io::stdout().write_all(svg.as_bytes()).unwrap_or_else(|e| {
                eprintln!("Failed to write stdout: {}", e);
                std::process::exit(1);
            });
        }
    }
}
