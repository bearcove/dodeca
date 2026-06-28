//! Browser asset lookup for disk-shipped JS/WASM bundles.
//!
//! These assets are intentionally not embedded into the `ddc` binary. In a
//! source checkout they are read from each package's generated `pkg/` or
//! `dist/` directory. In packaged installs they are read from `dodeca-assets/`
//! next to the binary, or one directory above the binary for package managers
//! that install `bin/ddc` under a prefix.

use std::path::{Path, PathBuf};

const ASSET_DIR: &str = "dodeca-assets";

const DEVTOOLS_RUNTIME_DIR: &str = "devtools-runtime";
const DEVTOOLS_UI_DIR: &str = "devtools-ui";
const SEARCH_DIR: &str = "search";

const DEVTOOLS_JS: &str = "dodeca_devtools.js";
const DEVTOOLS_WASM: &str = "dodeca_devtools_bg.wasm";

const SEARCH_JS: &str = "search.js";
const SEARCH_CSS: &str = "search.css";
const SEARCH_WASM_JS: &str = "dodeca_search_wasm.js";
const SEARCH_WASM: &str = "dodeca_search_wasm_bg.wasm";

#[derive(Debug, Clone)]
pub struct BrowserAsset {
    pub name: &'static str,
    pub bytes: Vec<u8>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/dodeca lives under <repo>/crates/dodeca")
        .to_path_buf()
}

fn packaged_asset_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("DODECA_ASSETS_DIR") {
        roots.push(PathBuf::from(root));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        roots.push(bin_dir.join(ASSET_DIR));
        if let Some(prefix_dir) = bin_dir.parent() {
            roots.push(prefix_dir.join(ASSET_DIR));
        }
    }
    roots
}

fn first_existing_file(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_file())
}

fn first_existing_dir(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_dir())
}

fn read_file(path: PathBuf) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn devtools_runtime_path(name: &str) -> Option<PathBuf> {
    let packaged = packaged_asset_roots()
        .into_iter()
        .map(|root| root.join(DEVTOOLS_RUNTIME_DIR).join(name));
    let source = std::iter::once(repo_root().join("crates/dodeca-devtools/pkg").join(name));
    first_existing_file(packaged.chain(source))
}

pub fn read_devtools_runtime_asset(name: &str) -> Option<Vec<u8>> {
    match name {
        DEVTOOLS_JS | DEVTOOLS_WASM => read_file(devtools_runtime_path(name)?),
        _ => None,
    }
}

pub fn devtools_runtime_assets() -> Option<[BrowserAsset; 2]> {
    Some([
        BrowserAsset {
            name: DEVTOOLS_JS,
            bytes: read_devtools_runtime_asset(DEVTOOLS_JS)?,
        },
        BrowserAsset {
            name: DEVTOOLS_WASM,
            bytes: read_devtools_runtime_asset(DEVTOOLS_WASM)?,
        },
    ])
}

pub fn devtools_ui_dir() -> Option<PathBuf> {
    let packaged = packaged_asset_roots()
        .into_iter()
        .map(|root| root.join(DEVTOOLS_UI_DIR));
    let source = std::iter::once(repo_root().join("crates/dodeca/devtools-ui/dist"));
    first_existing_dir(packaged.chain(source))
}

pub fn read_devtools_ui_asset(rel: &str) -> Option<Vec<u8>> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return None;
    }
    read_file(devtools_ui_dir()?.join(rel))
}

fn search_packaged_path(name: &str) -> Option<PathBuf> {
    first_existing_file(
        packaged_asset_roots()
            .into_iter()
            .map(|root| root.join(SEARCH_DIR).join(name)),
    )
}

fn search_source_path(name: &str) -> Option<PathBuf> {
    let root = repo_root();
    match name {
        SEARCH_JS | SEARCH_CSS => Some(root.join("crates/dodeca-search-wasm/ui").join(name)),
        SEARCH_WASM_JS | SEARCH_WASM => Some(root.join("crates/dodeca-search-wasm/pkg").join(name)),
        _ => None,
    }
}

fn search_runtime_path(name: &str) -> Option<PathBuf> {
    search_packaged_path(name).or_else(|| first_existing_file([search_source_path(name)?]))
}

pub fn read_search_runtime_asset(name: &str) -> Option<Vec<u8>> {
    match name {
        SEARCH_JS | SEARCH_CSS | SEARCH_WASM_JS | SEARCH_WASM => {
            read_file(search_runtime_path(name)?)
        }
        _ => None,
    }
}

pub fn search_runtime_assets() -> Option<[BrowserAsset; 4]> {
    Some([
        BrowserAsset {
            name: SEARCH_JS,
            bytes: read_search_runtime_asset(SEARCH_JS)?,
        },
        BrowserAsset {
            name: SEARCH_CSS,
            bytes: read_search_runtime_asset(SEARCH_CSS)?,
        },
        BrowserAsset {
            name: SEARCH_WASM_JS,
            bytes: read_search_runtime_asset(SEARCH_WASM_JS)?,
        },
        BrowserAsset {
            name: SEARCH_WASM,
            bytes: read_search_runtime_asset(SEARCH_WASM)?,
        },
    ])
}
