//! Build script for dodeca
//!
//! - Compiles WASM clients (devtools + search query core)
//! - Generates Styx schema from DodecaConfig

use std::process::Command;

fn main() {
    println!("cargo::rerun-if-env-changed=DODECA_RELEASE_VERSION");

    // Build devtools WASM (replaces livereload-client)
    build_wasm_crate("dodeca-devtools");

    // Build the full-text search query core WASM.
    build_wasm_crate("dodeca-search-wasm");

    // Generate Styx schema from config types
    facet_styx::GenerateSchema::<dodeca_config::DodecaConfig>::new()
        .crate_name("dodeca-config")
        .version("1")
        .cli("ddc")
        .write("schema.styx");

    // Generate the browser editor's TypeScript vox client + build the editor.
    generate_editor_client();
    build_editor();
}

/// Generate the TypeScript vox client for the in-browser editor from
/// `DevtoolsService`'s descriptor — the same generator vox uses for its own
/// clients. Written into the editor's source tree (write-if-changed, so we
/// don't retrigger the build), then bundled by vite.
fn generate_editor_client() {
    println!("cargo::rerun-if-changed=../dodeca-protocol/src/lib.rs");

    let descriptor = dodeca_protocol::devtools_service_service_descriptor();
    let ts = vox_codegen::targets::typescript::generate_service(descriptor);

    let path = std::path::Path::new("editor/src/devtools.generated.ts");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create editor/src");
    }
    let changed = std::fs::read_to_string(path)
        .map(|old| old != ts)
        .unwrap_or(true);
    if changed {
        std::fs::write(path, &ts).expect("write devtools.generated.ts");
    }
}

/// Build the vite/Monaco editor (`pnpm install` + `vite build`) and emit an
/// asset table (`OUT_DIR/editor_assets.rs`) that the http cell embeds and serves
/// at `/_/edit/*`. Degrades to an empty table (with a warning) if node/pnpm are
/// unavailable, so dodeca still compiles — the editor route is just absent.
fn build_editor() {
    let editor = std::path::Path::new("editor");

    println!("cargo::rerun-if-changed=editor/src");
    println!("cargo::rerun-if-changed=editor/package.json");
    println!("cargo::rerun-if-changed=editor/vite.config.ts");

    let assets = if run_editor_build(editor) {
        collect_dist_assets(&editor.join("dist"))
    } else {
        Vec::new()
    };
    write_editor_assets(&assets);
}

/// `pnpm install` + `vite build` in the editor dir. Returns whether `dist/edit.js`
/// was produced. Invokes the local `vite` binary directly to bypass pnpm's
/// pre-run dependency check.
fn run_editor_build(editor: &std::path::Path) -> bool {
    if !editor.join("package.json").exists() {
        return false; // not scaffolded yet
    }
    // If the bundle is already built (CI builds it explicitly, with visible
    // output, via `pnpm install` + `pnpm run build` — pnpm's .bin layout in CI
    // differs from a direct `node_modules/.bin/vite` exec), just embed it.
    if editor.join("dist/edit.js").exists() {
        return true;
    }
    let have_pnpm = Command::new("pnpm")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !have_pnpm {
        println!("cargo::warning=pnpm not found; /_/edit/* (browser editor) will be unavailable");
        return false;
    }
    // Install only when needed. We check the resulting `vite` binary rather than
    // pnpm's exit code: pnpm exits non-zero on the (harmless) ignored esbuild
    // build script when run without a TTY, even though deps install fine.
    let vite = editor.join("node_modules/.bin/vite");
    if !vite.exists() {
        let _ = Command::new("pnpm")
            .current_dir(editor)
            .arg("install")
            .status();
    }
    if !vite.exists() {
        println!("cargo::warning=editor deps missing after `pnpm install`; skipping editor build");
        return false;
    }
    // Absolute path: a relative program is resolved from the child's `current_dir`,
    // which would double-nest to `editor/editor/...`.
    let vite = std::fs::canonicalize(&vite).expect("canonicalize vite binary");
    let build = Command::new(&vite)
        .current_dir(editor)
        .arg("build")
        .status();
    if !matches!(build, Ok(s) if s.success()) {
        println!("cargo::warning=editor `vite build` failed");
        return false;
    }
    editor.join("dist/edit.js").exists()
}

/// Collect `dist/**` as `(served_relative_path, absolute_path)` pairs.
fn collect_dist_assets(dist: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let dist = std::fs::canonicalize(dist).expect("canonicalize dist");
    let mut out = Vec::new();
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<(String, std::path::PathBuf)>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    walk(&dist, &dist, &mut out);
    out
}

/// Write `OUT_DIR/editor_assets.rs`: a `&[(&str, &[u8])]` table the server
/// `include!`s, mapping each `/_/edit/<path>` to its embedded bytes.
fn write_editor_assets(assets: &[(String, std::path::PathBuf)]) {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let mut src = String::from("pub static EDITOR_ASSETS: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in assets {
        src.push_str(&format!(
            "    ({:?}, include_bytes!({:?})),\n",
            rel,
            abs.to_str().expect("utf-8 path")
        ));
    }
    src.push_str("];\n");
    std::fs::write(std::path::Path::new(&out_dir).join("editor_assets.rs"), src)
        .expect("write editor_assets.rs");
}

fn build_wasm_crate(name: &str) {
    let crate_path = format!("../{name}");
    let pkg_dir = std::path::Path::new(&crate_path).join("pkg");

    // Re-run if the crate's sources change (whole src/ tree, not just lib.rs).
    println!("cargo::rerun-if-changed={crate_path}/src");
    println!("cargo::rerun-if-changed={crate_path}/Cargo.toml");

    // Compute expected output filename (crate name with - replaced by _)
    let output_name = name.replace('-', "_");
    let wasm_file = format!("{output_name}_bg.wasm");
    let output = pkg_dir.join(&wasm_file);

    // Skip the (slow) wasm-pack build only if the output exists AND is newer
    // than every source file in this crate. The old "skip if pkg/ exists at
    // all" check let a stale pkg/ ship: combined with CI caches that preserve
    // pkg/ across builds, that's how v0.14.4 shipped an out-of-date search
    // reader (index parse failed with "invalid UTF-8"). NOTE: this only sees
    // *this* crate's sources, not shared deps like dodeca-search-format, so CI
    // additionally `rm -rf`s pkg/ before a release build for a clean rebuild.
    if output.exists() && !wasm_sources_newer_than(&crate_path, &output) {
        return;
    }

    // Try to build with wasm-pack (use separate target dir to avoid deadlock)
    let status = Command::new("wasm-pack")
        .current_dir(&crate_path)
        .args(["build", "--target", "web", "--target-dir", "target-wasm"])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => println!("cargo::warning=wasm-pack build failed for {name}"),
        Err(_) => println!(
            "cargo::warning=wasm-pack not found. Run: wasm-pack build --target web {crate_path}"
        ),
    }
}

/// Whether any file under `<crate_path>/src` (recursive) or
/// `<crate_path>/Cargo.toml` has an mtime newer than `reference`. Conservative:
/// if `reference` can't be stat'd, returns true (forces a rebuild).
fn wasm_sources_newer_than(crate_path: &str, reference: &std::path::Path) -> bool {
    let Ok(ref_mtime) = reference.metadata().and_then(|m| m.modified()) else {
        return true;
    };
    let cargo_toml = std::path::Path::new(crate_path).join("Cargo.toml");
    let cargo_newer = cargo_toml
        .metadata()
        .and_then(|m| m.modified())
        .map(|t| t > ref_mtime)
        .unwrap_or(false);
    let src_newer = newest_mtime_under(&std::path::Path::new(crate_path).join("src"))
        .map(|t| t > ref_mtime)
        .unwrap_or(false);
    cargo_newer || src_newer
}

/// Newest file mtime under `dir` (recursive), or `None` if it's empty/missing.
fn newest_mtime_under(dir: &std::path::Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(mtime) = path.metadata().and_then(|m| m.modified()) {
                newest = Some(newest.map_or(mtime, |n| n.max(mtime)));
            }
        }
    }
    newest
}
