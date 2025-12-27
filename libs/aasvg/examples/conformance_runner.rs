//! Conformance test runner - reads ASCII art from stdin, outputs SVG to stdout
//!
//! Usage: cargo run --example conformance_runner < input.txt > output.svg

use aasvg::{render_with_options, RenderOptions};
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("Failed to read stdin");

    let options = RenderOptions::new().with_stretch(true);
    let svg = render_with_options(&input, &options);
    print!("{}", svg);
}
