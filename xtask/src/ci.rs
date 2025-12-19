//! CI workflow generation for GitHub Actions.
//!
//! This module provides typed representations of GitHub Actions workflow files
//! and generates the release workflow for dodeca.

#![allow(dead_code)] // Scaffolding for future CI features

use facet::Facet;
use indexmap::IndexMap;

// =============================================================================
// Configuration
// =============================================================================

/// Target platforms for CI and releases.
pub const TARGETS: &[Target] = &[
    Target {
        triple: "x86_64-unknown-linux-gnu",
        os: "ubuntu-24.04",
        runner: "depot-ubuntu-24.04-32",
        lib_ext: "so",
        lib_prefix: "lib",
        archive_ext: "tar.xz",
    },
    Target {
        triple: "aarch64-apple-darwin",
        os: "macos-15",
        runner: "depot-macos-15",
        lib_ext: "dylib",
        lib_prefix: "lib",
        archive_ext: "tar.xz",
    },
];

/// Discover cdylib plugins by scanning crates/dodeca-*/Cargo.toml for cdylib crate-type.
pub fn discover_cdylib_cells(repo_root: &Utf8Path) -> Vec<String> {
    let mut cells = Vec::new();
    let crates_dir = repo_root.join("crates");

    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.starts_with("dodeca-")
            {
                let cargo_toml = path.join("Cargo.toml");
                if let Ok(content) = std::fs::read_to_string(&cargo_toml)
                    && content.contains("cdylib")
                {
                    cells.push(name.to_string());
                }
            }
        }
    }

    cells.sort();
    cells
}

/// Discover rapace plugins by scanning cells/cell-*/Cargo.toml for `[[bin]]` sections.
/// Returns (package_name, binary_name) pairs.
pub fn discover_rapace_cells(repo_root: &Utf8Path) -> Vec<(String, String)> {
    let mut cells = Vec::new();
    let cells_dir = repo_root.join("mods");

    if let Ok(entries) = std::fs::read_dir(&cells_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                // Skip proto crates
                && name.starts_with("cell-")
                && !name.ends_with("-proto")
            {
                let cargo_toml = path.join("Cargo.toml");
                if let Ok(content) = std::fs::read_to_string(&cargo_toml)
                    && content.contains("[[bin]]")
                {
                    let bin_name = format!("dodeca-{}", name);
                    cells.push((name.to_string(), bin_name));
                }
            }
        }
    }

    cells.sort();
    cells
}

/// All plugins sorted alphabetically (package_name, binary_name).
pub const ALL_CELLS: &[(&str, &str)] = &[
    ("cell-arborium", "ddc-cell-arborium"),
    ("cell-code-execution", "ddc-cell-code-execution"),
    ("cell-css", "ddc-cell-css"),
    ("cell-fonts", "ddc-cell-fonts"),
    ("cell-html", "ddc-cell-html"),
    ("cell-html-diff", "ddc-cell-html-diff"),
    ("cell-http", "ddc-cell-http"),
    ("cell-image", "ddc-cell-image"),
    ("cell-js", "ddc-cell-js"),
    ("cell-jxl", "ddc-cell-jxl"),
    ("cell-linkcheck", "ddc-cell-linkcheck"),
    ("cell-markdown", "ddc-cell-markdown"),
    ("cell-minify", "ddc-cell-minify"),
    ("cell-pagefind", "ddc-cell-pagefind"),
    ("cell-sass", "ddc-cell-sass"),
    ("cell-svgo", "ddc-cell-svgo"),
    ("cell-tui", "ddc-cell-tui"),
    ("cell-webp", "ddc-cell-webp"),
];

/// Group plugins into chunks of N for parallel CI builds.
pub fn cell_groups(chunk_size: usize) -> Vec<(String, Vec<(&'static str, &'static str)>)> {
    ALL_CELLS
        .chunks(chunk_size)
        .enumerate()
        .map(|(i, chunk)| (format!("{}", i + 1), chunk.to_vec()))
        .collect()
}

/// A target platform configuration.
pub struct Target {
    pub triple: &'static str,
    pub os: &'static str,
    pub runner: &'static str,
    pub lib_ext: &'static str,
    pub lib_prefix: &'static str,
    pub archive_ext: &'static str,
}

impl Target {
    /// Get a short name for the target (e.g., "linux-x64").
    pub fn short_name(&self) -> &'static str {
        match self.triple {
            "x86_64-unknown-linux-gnu" => "linux-x64",
            "aarch64-unknown-linux-gnu" => "linux-arm64",
            "aarch64-apple-darwin" => "macos-arm64",
            "x86_64-pc-windows-msvc" => "windows-x64",
            _ => self.triple,
        }
    }
}

// =============================================================================
// GitHub Actions Workflow Schema
// =============================================================================

structstruck::strike! {
    /// A GitHub Actions workflow file.
    #[strikethrough[derive(Debug, Clone, Facet)]]
    #[facet(rename_all = "kebab-case")]
    pub struct Workflow {
        /// The name of the workflow displayed in the GitHub UI.
        pub name: String,

        /// The events that trigger the workflow.
        pub on: On,

        /// Permissions for the workflow.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub permissions: Option<IndexMap<String, String>>,

        /// Environment variables available to all jobs.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub env: Option<IndexMap<String, String>>,

        /// The jobs that make up the workflow.
        pub jobs: IndexMap<String, Job>,
    }
}

structstruck::strike! {
    /// Events that trigger a workflow.
    #[strikethrough[derive(Debug, Clone, Facet)]]
    #[facet(rename_all = "snake_case")]
    pub struct On {
        /// Trigger on push events.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub push: Option<pub struct PushTrigger {
            /// Tags to trigger on.
            #[facet(default, skip_serializing_if = Option::is_none)]
            pub tags: Option<Vec<String>>,
            /// Branches to trigger on.
            #[facet(default, skip_serializing_if = Option::is_none)]
            pub branches: Option<Vec<String>>,
        }>,

        /// Trigger on pull request events.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub pull_request: Option<pub struct PullRequestTrigger {
            /// Branches to trigger on.
            #[facet(default, skip_serializing_if = Option::is_none)]
            pub branches: Option<Vec<String>>,
        }>,

        /// Trigger on workflow dispatch (manual).
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub workflow_dispatch: Option<pub struct WorkflowDispatchTrigger {}>,
    }
}

structstruck::strike! {
    /// A job in a workflow.
    #[strikethrough[derive(Debug, Clone, Facet)]]
    #[facet(rename_all = "kebab-case")]
    pub struct Job {
        /// Display name for the job in the GitHub UI.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub name: Option<String>,

        /// The runner to use.
        pub runs_on: String,

        /// Maximum time in minutes for the job to run.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub timeout_minutes: Option<u32>,

        /// Jobs that must complete before this one.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub needs: Option<Vec<String>>,

        /// Condition for running this job.
        #[facet(default, skip_serializing_if = Option::is_none, rename = "if")]
        pub if_condition: Option<String>,

        /// Outputs from this job.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub outputs: Option<IndexMap<String, String>>,

        /// Environment variables for this job.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub env: Option<IndexMap<String, String>>,

        /// Allow job failure without failing the workflow.
        #[facet(default, skip_serializing_if = Option::is_none, rename = "continue-on-error")]
        pub continue_on_error: Option<bool>,

        /// The steps to run.
        pub steps: Vec<Step>,
    }
}

structstruck::strike! {
    /// A step in a job.
    #[strikethrough[derive(Debug, Clone, Facet)]]
    #[facet(rename_all = "kebab-case")]
    pub struct Step {
        /// The name of the step.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub name: Option<String>,

        /// Step ID for referencing outputs.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub id: Option<String>,

        /// Use a GitHub Action.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub uses: Option<String>,

        /// Run a shell command.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub run: Option<String>,

        /// Shell to use for run commands.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub shell: Option<String>,

        /// Inputs for the action.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub with: Option<IndexMap<String, String>>,

        /// Environment variables for this step.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub env: Option<IndexMap<String, String>>,
    }
}

// =============================================================================
// Helper constructors
// =============================================================================

impl Step {
    /// Create a step that uses a GitHub Action.
    pub fn uses(name: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            id: None,
            uses: Some(action.into()),
            run: None,
            shell: None,
            with: None,
            env: None,
        }
    }

    /// Create a step that runs a shell command.
    pub fn run(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            id: None,
            uses: None,
            run: Some(command.into()),
            shell: None,
            with: None,
            env: None,
        }
    }

    /// Set the step ID.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set the shell.
    pub fn shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = Some(shell.into());
        self
    }

    /// Add inputs to this step.
    pub fn with_inputs(
        mut self,
        inputs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let map: IndexMap<String, String> = inputs
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self.with = Some(map);
        self
    }

    /// Add environment variables to this step.
    pub fn with_env(
        mut self,
        env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let map: IndexMap<String, String> =
            env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self.env = Some(map);
        self
    }
}

impl Job {
    /// Create a new job.
    pub fn new(runs_on: impl Into<String>) -> Self {
        Self {
            name: None,
            runs_on: runs_on.into(),
            timeout_minutes: None,
            needs: None,
            if_condition: None,
            outputs: None,
            env: None,
            continue_on_error: None,
            steps: Vec::new(),
        }
    }

    /// Set the timeout in minutes.
    pub fn timeout(mut self, minutes: u32) -> Self {
        self.timeout_minutes = Some(minutes);
        self
    }

    /// Set the display name for this job.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add dependencies to this job.
    pub fn needs(mut self, deps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.needs = Some(deps.into_iter().map(Into::into).collect());
        self
    }

    /// Set the condition for running this job.
    pub fn if_condition(mut self, condition: impl Into<String>) -> Self {
        self.if_condition = Some(condition.into());
        self
    }

    /// Set outputs for this job.
    pub fn outputs(
        mut self,
        outputs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.outputs = Some(
            outputs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// Set environment variables for this job.
    pub fn env(
        mut self,
        env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.env = Some(env.into_iter().map(|(k, v)| (k.into(), v.into())).collect());
        self
    }

    /// Allow this job to fail without failing the workflow.
    pub fn continue_on_error(mut self, enabled: bool) -> Self {
        self.continue_on_error = Some(enabled);
        self
    }

    /// Add steps to this job.
    pub fn steps(mut self, steps: impl IntoIterator<Item = Step>) -> Self {
        self.steps = steps.into_iter().collect();
        self
    }
}

// =============================================================================
// Common step patterns
// =============================================================================

pub mod common {
    use super::*;

    pub fn checkout() -> Step {
        Step::uses("Checkout", "actions/checkout@v4")
    }

    pub fn install_rust() -> Step {
        Step::uses("Install Rust", "dtolnay/rust-toolchain@stable")
    }

    pub fn install_rust_with_target(target: &str) -> Step {
        Step::uses("Install Rust", "dtolnay/rust-toolchain@stable")
            .with_inputs([("targets", target)])
    }

    pub fn rust_cache() -> Step {
        Step::uses("Rust cache", "Swatinem/rust-cache@v2")
            .with_inputs([("cache-on-failure", "true")])
    }

    pub fn upload_artifact(name: impl Into<String>, path: impl Into<String>) -> Step {
        Step::uses("Upload artifact", "actions/upload-artifact@v4")
            .with_inputs([("name", name.into()), ("path", path.into())])
    }

    pub fn download_artifact(name: impl Into<String>, path: impl Into<String>) -> Step {
        Step::uses("Download artifact", "actions/download-artifact@v4")
            .with_inputs([("name", name.into()), ("path", path.into())])
    }

    pub fn download_all_artifacts(path: impl Into<String>) -> Step {
        Step::uses("Download all artifacts", "actions/download-artifact@v4").with_inputs([
            ("path", path.into()),
            ("pattern", "build-*".to_string()),
            ("merge-multiple", "true".to_string()),
        ])
    }
}

// =============================================================================
// CI workflow builder (for PRs and main branch)
// =============================================================================

/// CI runner configuration.
struct CiRunner {
    os: &'static str,
    runner: &'static str,
    wasm_install: &'static str,
}

const CI_LINUX: CiRunner = CiRunner {
    os: "linux",
    runner: "depot-ubuntu-24.04-32",
    wasm_install: "curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh",
};

const CI_MACOS: CiRunner = CiRunner {
    os: "macos",
    runner: "depot-macos-15",
    wasm_install: "brew install wasm-pack xz",
};

/// Build the unified CI workflow (runs on PRs, main branch, and tags).
///
/// Strategy:
/// - Fan-out: Build ddc + cell groups (3 cells per job) for each target platform
/// - Fan-in: Assemble archives after all cell groups complete
/// - Integration: Run integration tests
/// - Release: On tags, publish GitHub release
pub fn build_ci_workflow() -> Workflow {
    use common::*;

    let mut jobs = IndexMap::new();
    let groups = cell_groups(3); // 3 cells per group

    // Track jobs required before release (assemble + integration per target)
    let mut all_release_needs: Vec<String> = Vec::new();

    jobs.insert(
        "clippy".to_string(),
        Job::new(CI_LINUX.runner)
            .name("Clippy")
            .timeout(30)
            .continue_on_error(true)
            .steps([
                checkout(),
                Step::uses("Install Rust", "dtolnay/rust-toolchain@stable").with_inputs([
                    ("components", "clippy"),
                    ("targets", "wasm32-unknown-unknown"),
                ]),
                rust_cache(),
                Step::run(
                    "Clippy",
                    "cargo clippy --all-features --all-targets -- -D warnings",
                ),
            ]),
    );

    let wasm_job_id = "build-wasm".to_string();
    let wasm_artifact = "dodeca-devtools-wasm".to_string();
    jobs.insert(
        wasm_job_id.clone(),
        Job::new(CI_LINUX.runner)
            .name("Build WASM")
            .timeout(30)
            .steps([
                checkout(),
                Step::uses("Install Rust", "dtolnay/rust-toolchain@stable")
                    .with_inputs([("targets", "wasm32-unknown-unknown")]),
                rust_cache(),
                Step::run("Install wasm-pack", CI_LINUX.wasm_install),
                Step::run("Build WASM", "cargo xtask wasm"),
                upload_artifact(wasm_artifact.clone(), "crates/dodeca-devtools/pkg"),
            ]),
    );

    for target in TARGETS {
        let short = target.short_name();

        // Job 1: Build ddc (main binary)
        let ddc_job_id = format!("build-ddc-{short}");
        jobs.insert(
            ddc_job_id.clone(),
            Job::new(target.runner)
                .name(format!("Build ddc ({short})"))
                .timeout(30)
                .steps([
                    checkout(),
                    Step::uses("Install Rust", "dtolnay/rust-toolchain@stable"),
                    Step::uses("Install Rust (nightly)", "dtolnay/rust-toolchain@nightly"),
                    rust_cache(),
                    Step::run("Build ddc", "cargo build --release -p dodeca"),
                    // Only run binary unit tests here - integration tests (serve/) need cells
                    // and run in the integration phase after assembly
                    Step::run("Test ddc", "cargo test --release -p dodeca --bins"),
                    upload_artifact(format!("ddc-{short}"), "target/release/ddc"),
                ]),
        );

        // Jobs 2-N: Build cell groups in parallel
        let mut cell_group_jobs: Vec<String> = Vec::new();
        for (group_num, cells) in &groups {
            let group_job_id = format!("build-cells-{short}-{group_num}");

            let build_args: String = cells
                .iter()
                .map(|(pkg, bin)| format!("-p {pkg} --bin {bin}"))
                .collect::<Vec<_>>()
                .join(" ");

            let test_args: String = cells
                .iter()
                .map(|(pkg, _)| format!("-p {pkg}"))
                .collect::<Vec<_>>()
                .join(" ");

            let cell_names: String = cells
                .iter()
                .map(|(pkg, _)| *pkg)
                .collect::<Vec<_>>()
                .join(", ");

            // Collect binary paths for upload
            let binary_paths: String = cells
                .iter()
                .map(|(_, bin)| format!("target/release/{bin}"))
                .collect::<Vec<_>>()
                .join("\n");

            jobs.insert(
                group_job_id.clone(),
                Job::new(target.runner)
                    .name(format!("Build cells ({short}) [{cell_names}]"))
                    .timeout(30)
                    .steps([
                        checkout(),
                        install_rust(),
                        rust_cache(),
                        Step::run("Build cells", format!("cargo build --release {build_args}")),
                        Step::run("Test cells", format!("cargo test --release {test_args}")),
                        upload_artifact(format!("cells-{short}-{group_num}"), binary_paths),
                    ]),
            );

            cell_group_jobs.push(group_job_id);
        }

        // Assembly job: download all artifacts and create archive
        let assemble_job_id = format!("assemble-{short}");
        let cell_group_needs = cell_group_jobs.clone();
        let mut assemble_needs = vec![ddc_job_id.clone(), wasm_job_id.clone()];
        assemble_needs.extend(cell_group_needs.clone());

        jobs.insert(
            assemble_job_id.clone(),
            Job::new(target.runner)
                .name(format!("Assemble ({short})"))
                .timeout(30)
                .needs(assemble_needs)
                .steps([
                    checkout(),
                    // Download ddc binary
                    Step::uses("Download ddc", "actions/download-artifact@v4").with_inputs([
                        ("name", format!("ddc-{short}")),
                        ("path", "target/release".into()),
                    ]),
                    // Download all cell group artifacts
                    Step::uses("Download cells", "actions/download-artifact@v4").with_inputs([
                        ("pattern", format!("cells-{short}-*")),
                        ("path", "target/release".into()),
                        ("merge-multiple", "true".into()),
                    ]),
                    Step::uses("Download WASM", "actions/download-artifact@v4").with_inputs([
                        ("name", wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::run(
                        "Install xz (macOS)",
                        "if command -v brew >/dev/null 2>&1; then brew install xz; fi",
                    ),
                    Step::run("List binaries", "ls -la target/release/"),
                    Step::run(
                        "Assemble archive",
                        format!("bash scripts/assemble-archive.sh {}", target.triple),
                    ),
                    upload_artifact(
                        format!("build-{short}"),
                        format!("dodeca-{}.{}", target.triple, target.archive_ext),
                    ),
                ]),
        );
        all_release_needs.push(assemble_job_id.clone());

        // Integration tests
        let integration_job_id = format!("integration-{short}");
        let mut integration_needs = vec![ddc_job_id.clone(), wasm_job_id.clone()];
        integration_needs.extend(cell_group_needs);
        jobs.insert(
            integration_job_id.clone(),
            Job::new(target.runner)
                .name(format!("Integration ({short})"))
                .timeout(30)
                .needs(integration_needs)
                .steps([
                    checkout(),
                    Step::uses("Install Rust", "dtolnay/rust-toolchain@stable"),
                    rust_cache(),
                    Step::uses("Download ddc", "actions/download-artifact@v4")
                        .with_inputs([("name", format!("ddc-{short}")), ("path", "dist".into())]),
                    Step::uses("Download cells", "actions/download-artifact@v4").with_inputs([
                        ("pattern", format!("cells-{short}-*")),
                        ("path", "dist".into()),
                        ("merge-multiple", "true".into()),
                    ]),
                    Step::uses("Download WASM", "actions/download-artifact@v4").with_inputs([
                        ("name", wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::run(
                        "Prepare binaries",
                        "chmod +x dist/ddc dist/ddc-cell-* && ls -la dist/",
                    ),
                    Step::run(
                        "Run integration tests",
                        "cargo xtask integration --no-build",
                    )
                    .with_env([
                        ("DODECA_BIN", "${{ github.workspace }}/dist/ddc"),
                        ("DODECA_CELL_PATH", "${{ github.workspace }}/dist"),
                    ]),
                ]),
        );

        all_release_needs.push(integration_job_id);
    }

    // Release job (only on tags, after integration tests pass)
    jobs.insert(
        "release".into(),
        Job::new("ubuntu-latest")
            .name("Release")
            .timeout(30)
            .needs(all_release_needs)
            .if_condition("startsWith(github.ref, 'refs/tags/')")
            .env([
                ("GH_TOKEN", "${{ secrets.GITHUB_TOKEN }}"),
                ("HOMEBREW_TAP_TOKEN", "${{ secrets.HOMEBREW_TAP_TOKEN }}"),
            ])
            .steps([
                checkout(),
                download_all_artifacts("dist"),
                Step::run("List artifacts", "ls -laR dist/"),
                Step::run(
                    "Create GitHub Release",
                    r#"
gh release create "${{ github.ref_name }}" \
  --title "dodeca ${{ github.ref_name }}" \
  --generate-notes \
  dist/**/*.tar.xz dist/**/*.zip
"#
                    .trim(),
                )
                .shell("bash"),
                Step::run(
                    "Update Homebrew tap",
                    r#"bash scripts/update-homebrew.sh "${{ github.ref_name }}""#,
                )
                .shell("bash"),
            ]),
    );

    Workflow {
        name: "CI".into(),
        on: On {
            push: Some(PushTrigger {
                tags: Some(vec!["v*".into()]),
                branches: Some(vec!["main".into()]),
            }),
            pull_request: Some(PullRequestTrigger {
                branches: Some(vec!["main".into()]),
            }),
            workflow_dispatch: Some(WorkflowDispatchTrigger {}),
        },
        permissions: Some(
            [("contents", "write")]
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        ),
        env: Some(
            [("CARGO_TERM_COLOR", "always")]
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        ),
        jobs,
    }
}

// =============================================================================
// Generation
// =============================================================================

use camino::Utf8Path;
use miette::Result;

const GENERATED_HEADER: &str =
    "# GENERATED BY: cargo xtask ci\n# DO NOT EDIT - edit xtask/src/ci.rs instead\n\n";

// =============================================================================
// Installer scripts
// =============================================================================

/// Generate the shell installer script content.
pub fn generate_installer_script() -> String {
    let repo = "bearcove/dodeca";

    format!(
        r##"#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://raw.githubusercontent.com/{repo}/main/install.sh | sh

set -eu

REPO="{repo}"

# Detect platform (only linux-x64 and macos-arm64 are supported)
detect_platform() {{
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64) echo "x86_64-unknown-linux-gnu" ;;
                *) echo "Unsupported Linux architecture: $arch (only x86_64 supported)" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64) echo "aarch64-apple-darwin" ;;
                *) echo "Unsupported macOS architecture: $arch (only arm64 supported)" >&2; exit 1 ;;
            esac
            ;;
        *)
            echo "Unsupported OS: $os" >&2
            exit 1
            ;;
    esac
}}

# Get latest release version
get_latest_version() {{
    curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/'
}}

main() {{
    local platform version archive_name url install_dir

    platform="$(detect_platform)"
    version="${{DODECA_VERSION:-$(get_latest_version)}}"
    archive_name="dodeca-$platform.tar.xz"
    url="https://github.com/$REPO/releases/download/$version/$archive_name"
    install_dir="${{DODECA_INSTALL_DIR:-$HOME/.cargo/bin}}"

    echo "Installing dodeca $version for $platform..."
    echo "  Archive: $url"
    echo "  Install dir: $install_dir"

    # Create install directory
    mkdir -p "$install_dir"

    # Download and extract
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap "rm -rf '$tmpdir'" EXIT

    echo "Downloading..."
    curl -fsSL "$url" -o "$tmpdir/archive.tar.xz"

    echo "Extracting..."
    tar -xJf "$tmpdir/archive.tar.xz" -C "$tmpdir"

    echo "Installing..."
    # Copy main binary
    cp "$tmpdir/ddc" "$install_dir/"
    chmod +x "$install_dir/ddc"

    # Copy cell binaries (ddc-cell-*)
    for plugin in "$tmpdir"/ddc-cell-*; do
        if [ -f "$plugin" ]; then
            cp "$plugin" "$install_dir/"
            chmod +x "$install_dir/$(basename "$plugin")"
        fi
    done

    echo ""
    echo "Successfully installed dodeca to $install_dir/ddc"
    echo ""

    # Check if install_dir is in PATH
    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *)
            echo "NOTE: $install_dir is not in your PATH."
            echo "Add this to your shell profile:"
            echo ""
            echo "  export PATH=\"\$PATH:$install_dir\""
            echo ""
            ;;
    esac
}}

main "$@"
"##,
        repo = repo
    )
}

/// Generate the PowerShell installer script content.
pub fn generate_powershell_installer() -> String {
    let repo = "bearcove/dodeca";

    format!(
        r##"# Installer for dodeca
# Usage: powershell -ExecutionPolicy Bypass -c "irm https://github.com/{repo}/releases/latest/download/dodeca-installer.ps1 | iex"

$ErrorActionPreference = 'Stop'

$REPO = "{repo}"

function Get-Architecture {{
    $arch = [System.Environment]::Is64BitOperatingSystem
    if ($arch) {{
        return "x86_64"
    }} else {{
        Write-Error "Only x64 architecture is supported on Windows"
        exit 1
    }}
}}

function Get-LatestVersion {{
    try {{
        $response = Invoke-RestMethod -Uri "https://api.github.com/repos/$REPO/releases/latest"
        return $response.tag_name
    }} catch {{
        Write-Error "Failed to get latest version: $_"
        exit 1
    }}
}}

function Main {{
    $arch = Get-Architecture
    $version = if ($env:DODECA_VERSION) {{ $env:DODECA_VERSION }} else {{ Get-LatestVersion }}
    $archiveName = "dodeca-x86_64-pc-windows-msvc.zip"
    $url = "https://github.com/$REPO/releases/download/$version/$archiveName"

    # Default install location
    $installDir = if ($env:DODECA_INSTALL_DIR) {{
        $env:DODECA_INSTALL_DIR
    }} else {{
        Join-Path $env:LOCALAPPDATA "dodeca"
    }}

    Write-Host "Installing dodeca $version for Windows x64..."
    Write-Host "  Archive: $url"
    Write-Host "  Install dir: $installDir"

    # Create install directory
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $pluginsDir = Join-Path $installDir "plugins"
    New-Item -ItemType Directory -Force -Path $pluginsDir | Out-Null

    # Download and extract
    $tempDir = Join-Path $env:TEMP "dodeca-install-$(New-Guid)"
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null

    try {{
        Write-Host "Downloading..."
        $archivePath = Join-Path $tempDir "archive.zip"
        Invoke-WebRequest -Uri $url -OutFile $archivePath

        Write-Host "Extracting..."
        Expand-Archive -Path $archivePath -DestinationPath $tempDir -Force

        Write-Host "Installing..."
        Copy-Item -Path (Join-Path $tempDir "ddc.exe") -Destination $installDir -Force

        $tempPluginsDir = Join-Path $tempDir "plugins"
        if (Test-Path $tempPluginsDir) {{
            Copy-Item -Path (Join-Path $tempPluginsDir "*") -Destination $pluginsDir -Force
        }}

        Write-Host ""
        Write-Host "Successfully installed dodeca to $installDir\ddc.exe"
        Write-Host ""

        # Check if install_dir is in PATH
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        if ($userPath -notlike "*$installDir*") {{
            Write-Host "NOTE: $installDir is not in your PATH."
            Write-Host "Adding $installDir to your user PATH..."

            try {{
                $newPath = if ($userPath) {{ "$userPath;$installDir" }} else {{ $installDir }}
                [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
                Write-Host "Successfully added to PATH. You may need to restart your terminal."
            }} catch {{
                Write-Host "Failed to add to PATH automatically. Please add it manually:"
                Write-Host "  1. Open System Properties > Environment Variables"
                Write-Host "  2. Add '$installDir' to your user PATH variable"
            }}
            Write-Host ""
        }}
    }} finally {{
        # Cleanup
        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }}
}}

Main
"##,
        repo = repo
    )
}

/// Helper to serialize a workflow to YAML with the generated header.
fn workflow_to_yaml(workflow: &Workflow) -> Result<String> {
    Ok(format!(
        "{}{}",
        GENERATED_HEADER,
        facet_yaml::to_string(workflow)
            .map_err(|e| miette::miette!("failed to serialize workflow: {}", e))?
    ))
}

/// Check or write a generated file.
fn check_or_write(path: &Utf8Path, content: &str, check: bool) -> Result<()> {
    if check {
        let existing = fs_err::read_to_string(path)
            .map_err(|e| miette::miette!("failed to read {}: {}", path, e))?;

        if existing != content {
            return Err(miette::miette!(
                "{} is out of date. Run `cargo xtask ci` to update.",
                path.file_name().unwrap_or("file")
            ));
        }
        println!("{} is up to date.", path.file_name().unwrap_or("file"));
    } else {
        fs_err::create_dir_all(path.parent().unwrap())
            .map_err(|e| miette::miette!("failed to create directory: {}", e))?;

        fs_err::write(path, content)
            .map_err(|e| miette::miette!("failed to write {}: {}", path, e))?;

        println!("Generated: {}", path);
    }
    Ok(())
}

/// Generate CI workflow and installer script.
pub fn generate(repo_root: &Utf8Path, check: bool) -> Result<()> {
    let workflows_dir = repo_root.join(".github/workflows");

    // Generate unified CI workflow (includes release on tags)
    let ci_workflow = build_ci_workflow();
    let ci_yaml = workflow_to_yaml(&ci_workflow)?;
    check_or_write(&workflows_dir.join("ci.yml"), &ci_yaml, check)?;

    // Generate installer script
    let installer_content = generate_installer_script();
    check_or_write(&repo_root.join("install.sh"), &installer_content, check)?;

    Ok(())
}
