//! Build script for dodeca
//!
//! - Generates Styx schema from DodecaConfig
//! - Generates the DevTools TypeScript vox bindings

fn main() {
    println!("cargo::rerun-if-env-changed=DODECA_RELEASE_VERSION");

    // Generate Styx schema from config types
    facet_styx::GenerateSchema::<dodeca_config::DodecaConfig>::new()
        .crate_name("dodeca-config")
        .version("1")
        .cli("ddc")
        .write("schema.styx");

    // Generate the DevTools bundle's TypeScript vox bindings.
    println!("cargo::rerun-if-changed=../dodeca-protocol/src/lib.rs");
    generate_bundle_bindings("devtools-ui");
}

/// Generate the TypeScript vox bindings (DevTools + Browser services) into a
/// bundle's source tree from the protocol descriptors — the same generator vox
/// uses for its own clients. Write-if-changed so we don't retrigger the build.
fn generate_bundle_bindings(dir: &str) {
    write_generated_ts(
        &format!("{dir}/src/devtools.generated.ts"),
        vox_codegen::targets::typescript::generate_service(
            dodeca_protocol::devtools_service_service_descriptor(),
        ),
    );
    write_generated_ts(
        &format!("{dir}/src/browser.generated.ts"),
        vox_codegen::targets::typescript::generate_service(
            dodeca_protocol::browser_service_service_descriptor(),
        ),
    );
}

fn write_generated_ts(path: &str, ts: String) {
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create bundle src dir");
    }
    let changed = std::fs::read_to_string(path)
        .map(|old| old != ts)
        .unwrap_or(true);
    if changed {
        std::fs::write(path, &ts).expect("write generated TypeScript bindings");
    }
}
