//! Build script to compile WASM clients (livereload + devtools)
//!
//! Run `wasm-pack build --target web crates/dodeca-devtools` before first build,
//! or this script will attempt to do it (requires wasm-pack installed).

use std::process::Command;

fn main() {
    // Build devtools WASM (replaces livereload-client)
    build_wasm_crate("dodeca-devtools");
}

fn build_wasm_crate(name: &str) {
    let crate_path = format!("crates/{name}");
    let pkg_dir = std::path::Path::new(&crate_path).join("pkg");

    // Re-run if the source changes
    println!("cargo::rerun-if-changed={crate_path}/src/lib.rs");
    println!("cargo::rerun-if-changed={crate_path}/Cargo.toml");

    // Compute expected output filename (crate name with - replaced by _)
    let output_name = name.replace('-', "_");
    let js_file = format!("{output_name}.js");

    // If pkg already exists, we're good
    if pkg_dir.join(&js_file).exists() {
        return;
    }

    // Try to build with wasm-pack
    let status = Command::new("wasm-pack")
        .args(["build", "--target", "web", &crate_path])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => println!("cargo::warning=wasm-pack build failed for {name}"),
        Err(_) => println!(
            "cargo::warning=wasm-pack not found. Run: wasm-pack build --target web {crate_path}"
        ),
    }
}
