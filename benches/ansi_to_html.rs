//! Benchmarks for ANSI to HTML conversion
//!
//! Run with: cargo bench --bench ansi_to_html

use divan::{black_box, Bencher};
use dodeca::error_pages::ansi_to_html;

fn main() {
    divan::main();
}

// ============================================================================
// Test data generators
// ============================================================================

/// Plain text without any ANSI codes
fn generate_plain_text(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("This is line {} of plain text without any formatting.\n", i))
        .collect()
}

/// Text with basic ANSI formatting (bold, colors)
fn generate_basic_ansi(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            format!(
                "\x1b[1mBold line {}\x1b[0m: \x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m \x1b[34mblue\x1b[0m\n",
                i
            )
        })
        .collect()
}

/// Simulated compiler error output with complex formatting
fn generate_compiler_error() -> String {
    r#"
error[E0382]: borrow of moved value: `data`
   --> src/main.rs:15:20
    |
12  |     let data = vec![1, 2, 3];
    |         ---- move occurs because `data` has type `Vec<i32>`, which does not implement the `Copy` trait
13  |     let result = process(data);
    |                          ---- value moved here
14  |
15  |     println!("{:?}", data);
    |                      ^^^^ value borrowed here after move
    |
note: consider cloning the value if the performance cost is acceptable
   --> src/main.rs:13:26
    |
13  |     let result = process(data);
    |                          ^^^^
help: consider cloning the value
    |
13  |     let result = process(data.clone());
    |                              ++++++++

For more information about this error, try `rustc --explain E0382`.
"#.replace("error", "\x1b[1;31merror\x1b[0m")
     .replace("-->", "\x1b[34m-->\x1b[0m")
     .replace("note:", "\x1b[1;36mnote:\x1b[0m")
     .replace("help:", "\x1b[1;32mhelp:\x1b[0m")
     .replace("^^^^", "\x1b[1;31m^^^^\x1b[0m")
     .replace("----", "\x1b[34m----\x1b[0m")
}

/// Text with 24-bit RGB colors
fn generate_rgb_colors(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            let r = (i * 10) % 256;
            let g = (i * 20) % 256;
            let b = (i * 30) % 256;
            format!(
                "\x1b[38;2;{};{};{}mLine {} with RGB color ({}, {}, {})\x1b[0m\n",
                r, g, b, i, r, g, b
            )
        })
        .collect()
}

/// Deeply nested formatting (multiple attributes)
fn generate_nested_formatting(lines: usize) -> String {
    (0..lines)
        .map(|i| {
            format!(
                "\x1b[1m\x1b[4m\x1b[31mBold underline red line {}\x1b[0m normal \x1b[2m\x1b[3mdim italic\x1b[0m\n",
                i
            )
        })
        .collect()
}

// ============================================================================
// Benchmarks
// ============================================================================

#[divan::bench]
fn plain_text_small(bencher: Bencher) {
    let input = generate_plain_text(10);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn plain_text_large(bencher: Bencher) {
    let input = generate_plain_text(1000);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn basic_ansi_small(bencher: Bencher) {
    let input = generate_basic_ansi(10);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn basic_ansi_large(bencher: Bencher) {
    let input = generate_basic_ansi(1000);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn compiler_error(bencher: Bencher) {
    let input = generate_compiler_error();
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn rgb_colors_small(bencher: Bencher) {
    let input = generate_rgb_colors(10);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn rgb_colors_large(bencher: Bencher) {
    let input = generate_rgb_colors(500);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn nested_formatting_small(bencher: Bencher) {
    let input = generate_nested_formatting(10);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

#[divan::bench]
fn nested_formatting_large(bencher: Bencher) {
    let input = generate_nested_formatting(500);
    bencher.bench(|| black_box(ansi_to_html(black_box(&input))));
}

// ============================================================================
// Throughput benchmarks
// ============================================================================

#[divan::bench(args = [1024, 10240, 102400])]
fn throughput_bytes(bencher: Bencher, size: usize) {
    // Generate text of approximately the given size
    let line = "\x1b[31mError:\x1b[0m Something went \x1b[1mwrong\x1b[0m at line 42\n";
    let repetitions = size / line.len() + 1;
    let input: String = std::iter::repeat_n(line, repetitions).collect();

    bencher
        .counter(divan::counter::BytesCount::new(input.len()))
        .bench(|| black_box(ansi_to_html(black_box(&input))));
}
