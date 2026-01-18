//! Build script for dodeca
//!
//! - Compiles WASM clients (livereload + devtools)
//! - Generates Styx schema from DodecaConfig

use std::process::Command;

fn main() {
    // Build devtools WASM (replaces livereload-client)
    build_wasm_crate("dodeca-devtools");

    // Generate Styx schema from config types
    facet_styx::GenerateSchema::<dodeca_config::DodecaConfig>::new()
        .crate_name("dodeca-config")
        .version("1")
        .cli("ddc")
        .write("schema.styx");
}

fn build_wasm_crate(name: &str) {
    let crate_path = format!("../{name}");
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

    // Try to build with wasm-pack (use separate target dir to avoid deadlock)
    let status = Command::new("wasm-pack")
        .args([
            "build",
            "--target",
            "web",
            "--target-dir",
            "target-wasm",
            &crate_path,
        ])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => println!("cargo::warning=wasm-pack build failed for {name}"),
        Err(_) => println!(
            "cargo::warning=wasm-pack not found. Run: wasm-pack build --target web {crate_path}"
        ),
    }
}
