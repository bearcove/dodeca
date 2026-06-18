//! Render example.txt to example.svg

use std::fs;

fn main() {
    let input = fs::read_to_string("example.txt").expect("Failed to read example.txt");
    let svg = aasvg::render(&input);
    fs::write("example.svg", &svg).expect("Failed to write example.svg");
    println!("Generated example.svg");
}
