//! Browser asset lookup for disk-shipped JS/WASM bundles.
//!
//! These assets are intentionally not embedded into the `ddc` binary. In a
//! source checkout they are read from each package's generated `pkg/` or
//! `dist/` directory. In packaged installs they are read from `dodeca-assets/`
//! next to the binary, or one directory above the binary for package managers
//! that install `bin/ddc` under a prefix.

use std::path::{Path, PathBuf};

use facet::Facet;

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

#[derive(Debug, Clone, Facet)]
pub struct BrowserAssetReport {
    pub ok: bool,
    pub production_ok: bool,
    pub source_fallback: bool,
    pub lookup_roots: Vec<BrowserAssetLookupRoot>,
    pub groups: Vec<BrowserAssetGroupReport>,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, Facet)]
pub struct BrowserAssetLookupRoot {
    pub label: String,
    pub path: String,
    pub active: bool,
    pub exists: bool,
}

#[derive(Debug, Clone, Facet)]
pub struct BrowserAssetGroupReport {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub required_for: Vec<String>,
    pub ok: bool,
    pub files: Vec<BrowserAssetFileReport>,
}

#[derive(Debug, Clone, Facet)]
pub struct BrowserAssetFileReport {
    pub name: String,
    pub status: String,
    pub path: Option<String>,
    pub size: Option<u64>,
    pub error: Option<String>,
    pub tried: Vec<String>,
}

#[derive(Debug, Clone)]
struct AssetRootCandidate {
    label: &'static str,
    path: PathBuf,
    active: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct BrowserAssetReportOptions {
    pub source_fallback: bool,
}

impl Default for BrowserAssetReportOptions {
    fn default() -> Self {
        Self {
            source_fallback: true,
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/dodeca lives under <repo>/crates/dodeca")
        .to_path_buf()
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn packaged_asset_root_candidates() -> Vec<AssetRootCandidate> {
    let mut roots = Vec::new();

    match std::env::var_os("DODECA_ASSETS_DIR") {
        Some(root) => roots.push(AssetRootCandidate {
            label: "DODECA_ASSETS_DIR",
            path: PathBuf::from(root),
            active: true,
        }),
        None => roots.push(AssetRootCandidate {
            label: "DODECA_ASSETS_DIR",
            path: PathBuf::from("<unset>"),
            active: false,
        }),
    }

    match std::env::current_exe() {
        Ok(exe) => {
            if let Some(bin_dir) = exe.parent() {
                roots.push(AssetRootCandidate {
                    label: "next to current executable",
                    path: bin_dir.join(ASSET_DIR),
                    active: true,
                });
                if let Some(prefix_dir) = bin_dir.parent() {
                    roots.push(AssetRootCandidate {
                        label: "install prefix",
                        path: prefix_dir.join(ASSET_DIR),
                        active: true,
                    });
                }
            }
        }
        Err(_) => roots.push(AssetRootCandidate {
            label: "current executable",
            path: PathBuf::from("<unavailable>"),
            active: false,
        }),
    }

    roots
}

fn packaged_asset_roots() -> Vec<PathBuf> {
    packaged_asset_root_candidates()
        .into_iter()
        .filter(|root| root.active)
        .map(|root| root.path)
        .collect()
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

fn devtools_runtime_candidates(name: &str) -> Vec<PathBuf> {
    devtools_runtime_candidates_for(name, true)
}

fn devtools_runtime_candidates_for(name: &str, source_fallback: bool) -> Vec<PathBuf> {
    let packaged = packaged_asset_roots()
        .into_iter()
        .map(|root| root.join(DEVTOOLS_RUNTIME_DIR).join(name));
    let source = source_fallback
        .then(|| repo_root().join("crates/dodeca-devtools/pkg").join(name))
        .into_iter();
    packaged.chain(source).collect()
}

fn devtools_runtime_path(name: &str) -> Option<PathBuf> {
    first_existing_file(devtools_runtime_candidates(name))
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

fn devtools_ui_candidates_for(name: &str, source_fallback: bool) -> Vec<PathBuf> {
    let packaged = packaged_asset_roots()
        .into_iter()
        .map(|root| root.join(DEVTOOLS_UI_DIR).join(name));
    let source = source_fallback
        .then(|| {
            repo_root()
                .join("crates/dodeca/devtools-ui/dist")
                .join(name)
        })
        .into_iter();
    packaged.chain(source).collect()
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

fn search_runtime_candidates_for(name: &str, source_fallback: bool) -> Vec<PathBuf> {
    let packaged = packaged_asset_roots()
        .into_iter()
        .map(|root| root.join(SEARCH_DIR).join(name));
    let source = if source_fallback {
        search_source_path(name)
    } else {
        None
    };
    packaged.chain(source).collect()
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
    search_packaged_path(name).or_else(|| first_existing_file(search_source_path(name)))
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

pub fn report() -> BrowserAssetReport {
    report_with_options(BrowserAssetReportOptions::default())
}

pub fn report_with_options(options: BrowserAssetReportOptions) -> BrowserAssetReport {
    let groups = vec![
        group_report(
            "devtools-runtime",
            "DevTools runtime",
            "live reload, DOM patching, CSS reload, and source-open helpers",
            &["serve"],
            vec![
                (
                    DEVTOOLS_JS,
                    devtools_runtime_candidates_for(DEVTOOLS_JS, options.source_fallback),
                ),
                (
                    DEVTOOLS_WASM,
                    devtools_runtime_candidates_for(DEVTOOLS_WASM, options.source_fallback),
                ),
            ],
        ),
        group_report(
            "devtools-ui",
            "DevTools UI",
            "the browser editor, annotation UI, and DevTools panel assets",
            &["serve"],
            vec![
                (
                    "devtools.js",
                    devtools_ui_candidates_for("devtools.js", options.source_fallback),
                ),
                (
                    "devtools.css",
                    devtools_ui_candidates_for("devtools.css", options.source_fallback),
                ),
            ],
        ),
        group_report(
            "search-runtime",
            "Search runtime",
            "the production search widget and browser query WASM",
            &["build", "serve"],
            vec![
                (
                    SEARCH_JS,
                    search_runtime_candidates_for(SEARCH_JS, options.source_fallback),
                ),
                (
                    SEARCH_CSS,
                    search_runtime_candidates_for(SEARCH_CSS, options.source_fallback),
                ),
                (
                    SEARCH_WASM_JS,
                    search_runtime_candidates_for(SEARCH_WASM_JS, options.source_fallback),
                ),
                (
                    SEARCH_WASM,
                    search_runtime_candidates_for(SEARCH_WASM, options.source_fallback),
                ),
            ],
        ),
    ];

    let production_ok = groups
        .iter()
        .find(|group| group.id == "search-runtime")
        .map(|group| group.ok)
        .unwrap_or(false);
    let ok = groups.iter().all(|group| group.ok);

    BrowserAssetReport {
        ok,
        production_ok,
        source_fallback: options.source_fallback,
        lookup_roots: lookup_root_report(options.source_fallback),
        groups,
        instructions: vec![
            "Source checkout: run `scripts/build-browser-assets.sh` to create the JS/WASM outputs."
                .to_string(),
            "Package layout: run `scripts/stage-browser-assets.sh <dist>/dodeca-assets` and ship that directory beside `ddc`."
                .to_string(),
            "Release archives: run `scripts/assemble-archive.sh <target-triple>`; it stages `dodeca-assets/` and fails when inputs are missing."
                .to_string(),
            "Custom installs: set `DODECA_ASSETS_DIR=/absolute/path/to/dodeca-assets` when assets are not next to the executable."
                .to_string(),
            "CI split: keep `cargo check` independent; build and stage browser assets only in browser-test and packaging jobs."
                .to_string(),
        ],
    }
}

pub fn render_markdown_report(report: &BrowserAssetReport) -> String {
    let mut out = String::new();
    out.push_str("# Dodeca Browser Assets\n\n");
    out.push_str(&format!(
        "- Overall: {}\n",
        if report.ok { "ok" } else { "incomplete" }
    ));
    out.push_str(&format!(
        "- Production build assets: {}\n\n",
        if report.production_ok {
            "ok"
        } else {
            "missing"
        }
    ));
    out.push_str(&format!(
        "- Source checkout fallback: {}\n\n",
        if report.source_fallback {
            "enabled"
        } else {
            "disabled"
        }
    ));

    out.push_str("## Groups\n\n");
    out.push_str("| Group | Required For | State | Files |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for group in &report.groups {
        let files = group
            .files
            .iter()
            .filter(|file| file.status != "ok")
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>();
        let file_cell = if files.is_empty() {
            "all present".to_string()
        } else {
            format!("missing: {}", files.join(", "))
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            md_cell(&group.name),
            md_cell(&group.required_for.join(", ")),
            if group.ok { "ok" } else { "incomplete" },
            md_cell(&file_cell)
        ));
    }

    out.push_str("\n## Lookup Roots\n\n");
    for root in &report.lookup_roots {
        out.push_str(&format!(
            "- `{}`: `{}` ({}, {})\n",
            root.label,
            root.path,
            if root.active { "active" } else { "inactive" },
            if root.exists { "exists" } else { "not found" }
        ));
    }

    out.push_str("\n## Files\n\n");
    for group in &report.groups {
        out.push_str(&format!("### {}\n\n", group.name));
        out.push_str(&format!("{}\n\n", group.purpose));
        for file in &group.files {
            out.push_str(&format!("- `{}`: {}", file.name, file.status));
            if let Some(path) = &file.path {
                out.push_str(&format!(" at `{path}`"));
            }
            if let Some(size) = file.size {
                out.push_str(&format!(" ({size} bytes)"));
            }
            if let Some(error) = &file.error {
                out.push_str(&format!("; {error}"));
            }
            out.push('\n');
            if file.status != "ok" {
                out.push_str("  Tried:\n");
                for path in &file.tried {
                    out.push_str(&format!("  - `{path}`\n"));
                }
            }
        }
        out.push('\n');
    }

    out.push_str("## Fix\n\n");
    for instruction in &report.instructions {
        out.push_str(&format!("- {instruction}\n"));
    }
    out.push_str(
        "- Verify an installed or staged layout with `DODECA_ASSETS_DIR=<dist>/dodeca-assets ddc assets --packaged --fail`.\n",
    );

    out
}

pub fn render_missing_summary(
    report: &BrowserAssetReport,
    production_only: bool,
) -> Option<String> {
    let missing = missing_groups(report, production_only);
    if missing.is_empty() {
        return None;
    }

    let scope = if production_only {
        "production browser assets"
    } else {
        "browser assets"
    };
    let mut out = String::new();
    out.push_str(&format!("Dodeca {scope} are incomplete.\n"));
    for group in missing {
        let missing_files = group
            .files
            .iter()
            .filter(|file| file.status != "ok")
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "- {}: missing or unreadable {}\n",
            group.name, missing_files
        ));
    }
    out.push_str("Run `ddc assets` for lookup paths and repair commands.\n");
    out.push_str("From a source checkout, run `scripts/build-browser-assets.sh`.\n");
    out.push_str(
        "For packaged installs, ship `dodeca-assets/` beside `ddc` or set `DODECA_ASSETS_DIR`.\n",
    );
    Some(out)
}

fn missing_groups(
    report: &BrowserAssetReport,
    production_only: bool,
) -> Vec<&BrowserAssetGroupReport> {
    report
        .groups
        .iter()
        .filter(|group| !group.ok)
        .filter(|group| {
            !production_only
                || group
                    .required_for
                    .iter()
                    .any(|required| required == "build")
        })
        .collect()
}

fn lookup_root_report(source_fallback: bool) -> Vec<BrowserAssetLookupRoot> {
    let mut roots = packaged_asset_root_candidates()
        .into_iter()
        .map(|root| BrowserAssetLookupRoot {
            label: root.label.to_string(),
            path: path_string(&root.path),
            active: root.active,
            exists: root.active && root.path.is_dir(),
        })
        .collect::<Vec<_>>();

    let repo = repo_root();
    roots.push(BrowserAssetLookupRoot {
        label: "source checkout".to_string(),
        path: path_string(&repo),
        active: source_fallback,
        exists: repo.is_dir(),
    });

    roots
}

fn group_report(
    id: &str,
    name: &str,
    purpose: &str,
    required_for: &[&str],
    files: Vec<(&'static str, Vec<PathBuf>)>,
) -> BrowserAssetGroupReport {
    let files = files
        .into_iter()
        .map(|(name, candidates)| file_report(name, candidates))
        .collect::<Vec<_>>();
    let ok = files.iter().all(|file| file.status == "ok");
    BrowserAssetGroupReport {
        id: id.to_string(),
        name: name.to_string(),
        purpose: purpose.to_string(),
        required_for: required_for.iter().map(|value| value.to_string()).collect(),
        ok,
        files,
    }
}

fn file_report(name: &'static str, candidates: Vec<PathBuf>) -> BrowserAssetFileReport {
    let tried = candidates
        .iter()
        .map(|path| path_string(path))
        .collect::<Vec<_>>();

    for path in candidates {
        if !path.is_file() {
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => {
                return BrowserAssetFileReport {
                    name: name.to_string(),
                    status: "ok".to_string(),
                    path: Some(path_string(&path)),
                    size: Some(bytes.len() as u64),
                    error: None,
                    tried,
                };
            }
            Err(error) => {
                return BrowserAssetFileReport {
                    name: name.to_string(),
                    status: "unreadable".to_string(),
                    path: Some(path_string(&path)),
                    size: None,
                    error: Some(error.to_string()),
                    tried,
                };
            }
        }
    }

    BrowserAssetFileReport {
        name: name.to_string(),
        status: "missing".to_string(),
        path: None,
        size: None,
        error: None,
        tried,
    }
}

fn md_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}
