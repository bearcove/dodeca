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

/// Target platforms for releases.
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
        triple: "aarch64-unknown-linux-gnu",
        os: "ubuntu-24.04",
        runner: "depot-ubuntu-24.04-arm-32",
        lib_ext: "so",
        lib_prefix: "lib",
        archive_ext: "tar.xz",
    },
    Target {
        triple: "aarch64-apple-darwin",
        os: "macos-latest",
        runner: "depot-macos-latest",
        lib_ext: "dylib",
        lib_prefix: "lib",
        archive_ext: "tar.xz",
    },
    Target {
        triple: "x86_64-pc-windows-msvc",
        os: "windows-latest",
        runner: "depot-windows-2022-16",
        lib_ext: "dll",
        lib_prefix: "",
        archive_ext: "zip",
    },
];

/// Plugin crates to build.
pub const PLUGINS: &[&str] = &[
    "dodeca-baseline",
    "dodeca-css",
    "dodeca-fonts",
    "dodeca-html-diff",
    "dodeca-image",
    "dodeca-js",
    "dodeca-jxl",
    "dodeca-linkcheck",
    "dodeca-minify",
    "dodeca-pagefind",
    "dodeca-sass",
    "dodeca-svgo",
    "dodeca-webp",
];

/// Plugin groups for parallel CI builds.
/// Groups are designed to balance build times and minimize dependency overlap.
pub struct PluginGroup {
    pub name: &'static str,
    pub plugins: &'static [&'static str],
}

/// Plugin groups:
/// - group1 (image processing): webp, jxl, image - share image codec dependencies
/// - group2 (web assets): minify, css, sass, html-diff - lightweight web processing
/// - group3 (misc): fonts, pagefind, linkcheck, svgo, baseline
/// - group4 (js): js - isolated because OXC is a heavy dependency
/// - group5 (code): code-execution - isolated due to pulldown-cmark
pub const PLUGIN_GROUPS: &[PluginGroup] = &[
    PluginGroup {
        name: "image",
        plugins: &["dodeca-webp", "dodeca-jxl", "dodeca-image"],
    },
    PluginGroup {
        name: "web",
        plugins: &["dodeca-minify", "dodeca-css", "dodeca-sass", "dodeca-html-diff"],
    },
    PluginGroup {
        name: "misc",
        plugins: &["dodeca-fonts", "dodeca-pagefind", "dodeca-linkcheck", "dodeca-svgo", "dodeca-baseline"],
    },
    PluginGroup {
        name: "js",
        plugins: &["dodeca-js"],
    },
    PluginGroup {
        name: "code",
        plugins: &["dodeca-code-execution"],
    },
];

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
            needs: None,
            if_condition: None,
            outputs: None,
            env: None,
            steps: Vec::new(),
        }
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
        Step::uses("Download all artifacts", "actions/download-artifact@v4")
            .with_inputs([
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
    lib_ext: &'static str,
    lib_prefix: &'static str,
    wasm_install: &'static str,
}

const CI_LINUX: CiRunner = CiRunner {
    os: "linux",
    runner: "depot-ubuntu-24.04-16",
    lib_ext: "so",
    lib_prefix: "lib",
    wasm_install: "curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh",
};

const CI_MACOS: CiRunner = CiRunner {
    os: "macos",
    runner: "depot-macos-15",
    lib_ext: "dylib",
    lib_prefix: "lib",
    wasm_install: "brew install wasm-pack",
};

/// Build the CI workflow (runs on PRs and main branch pushes).
///
/// Strategy:
/// - Windows: Simple sequential build + check (no parallelization needed)
/// - Linux/macOS: Parallel build jobs, each with unit tests, uploading artifacts
///   - build-ddc: Builds ddc binary + WASM, runs unit tests, uploads binary
///   - build-plugins-{group}: Builds plugin group, runs unit tests, uploads .so/.dylib
///   - integration: Downloads all artifacts, runs integration tests
pub fn build_ci_workflow() -> Workflow {
    use common::*;

    let mut jobs = IndexMap::new();

    // Windows check job (simple, no parallelization needed)
    jobs.insert(
        "check-windows".into(),
        Job::new("depot-windows-2022-16")
            .name("Check (Windows)")
            .steps([
                checkout(),
                install_rust_with_target("wasm32-unknown-unknown"),
                rust_cache(),
                Step::run("Install wasm-pack", "cargo install wasm-pack").shell("bash"),
                Step::run("Build", "cargo xtask build").shell("bash"),
                Step::run("Check", "cargo check --all-features").shell("bash"),
            ]),
    );

    // For Linux and macOS, we use parallel builds with artifact passing
    for runner in [&CI_LINUX, &CI_MACOS] {
        let os = runner.os;
        let lib_ext = runner.lib_ext;
        let lib_prefix = runner.lib_prefix;

        // Job: Build ddc binary + WASM
        let build_ddc_id = format!("build-ddc-{os}");
        jobs.insert(
            build_ddc_id.clone(),
            Job::new(runner.runner)
                .name(format!("Build ddc ({os})"))
                .steps([
                    checkout(),
                    Step::uses("Install Rust (stable)", "dtolnay/rust-toolchain@stable")
                        .with_inputs([
                            ("components", "clippy"),
                            ("targets", "wasm32-unknown-unknown"),
                        ]),
                    Step::uses("Install Rust (nightly)", "dtolnay/rust-toolchain@nightly"),
                    rust_cache(),
                    Step::run("Install wasm-pack", runner.wasm_install),
                    Step::run("Build WASM", "cargo xtask wasm"),
                    Step::run("Build ddc", "cargo build --release -p dodeca"),
                    Step::run("Verify ddc binary exists", "ls -la target/release/ddc"),
                    Step::run("Run ddc unit tests", "cargo test --release -p dodeca --lib"),
                    Step::run("Clippy", "cargo clippy --all-features --all-targets -- -D warnings"),
                    upload_artifact(
                        format!("ddc-{os}"),
                        "target/release/ddc",
                    ),
                ]),
        );

        // Jobs: Build plugin groups in parallel
        let mut all_build_job_ids = vec![build_ddc_id];

        for group in PLUGIN_GROUPS {
            let job_id = format!("build-plugins-{}-{os}", group.name);
            let plugins_arg: String = group.plugins.iter()
                .map(|p| format!("-p {p}"))
                .collect::<Vec<_>>()
                .join(" ");
            let test_arg: String = group.plugins.iter()
                .map(|p| format!("-p {p}"))
                .collect::<Vec<_>>()
                .join(" ");

            // Build the glob pattern for uploading plugin .so/.dylib files
            let upload_paths: String = group.plugins.iter()
                .map(|p| {
                    let lib_name = p.replace('-', "_");
                    format!("target/release/{lib_prefix}{lib_name}.{lib_ext}")
                })
                .collect::<Vec<_>>()
                .join("\n");

            jobs.insert(
                job_id.clone(),
                Job::new(runner.runner)
                    .name(format!("Build plugins/{} ({os})", group.name))
                    .steps([
                        checkout(),
                        Step::uses("Install Rust", "dtolnay/rust-toolchain@stable"),
                        rust_cache(),
                        Step::run(
                            format!("Build {} plugins", group.name),
                            format!("cargo build --release {plugins_arg}"),
                        ),
                        Step::run(
                            format!("Test {} plugins", group.name),
                            format!("cargo test --release {test_arg}"),
                        ),
                        Step::uses("Upload plugin artifacts", "actions/upload-artifact@v4")
                            .with_inputs([
                                ("name", format!("plugins-{}-{os}", group.name)),
                                ("path", upload_paths),
                                ("retention-days", "1".into()),
                            ]),
                    ]),
            );

            all_build_job_ids.push(job_id);
        }

        // Job: Integration tests - download all artifacts and run integration tests
        jobs.insert(
            format!("integration-{os}"),
            Job::new(runner.runner)
                .name(format!("Integration ({os})"))
                .needs(all_build_job_ids)
                .steps([
                    checkout(),
                    Step::uses("Install Rust", "dtolnay/rust-toolchain@stable"),
                    rust_cache(),
                    // Download ddc binary
                    Step::uses("Download ddc", "actions/download-artifact@v4")
                        .with_inputs([
                            ("name", format!("ddc-{os}")),
                            ("path", "artifacts/bin".into()),
                        ]),
                    // Download all plugin artifacts
                    Step::uses("Download plugins", "actions/download-artifact@v4")
                        .with_inputs([
                            ("pattern", format!("plugins-*-{os}")),
                            ("path", "artifacts/plugins".into()),
                            ("merge-multiple", "true".into()),
                        ]),
                    Step::run("List artifacts", "ls -laR artifacts/"),
                    Step::run("Make ddc executable", "chmod +x artifacts/bin/ddc"),
                    Step::run(
                        "Run integration tests",
                        "cargo test --release -p dodeca-integration --test '*'",
                    )
                    .with_env([
                        ("DODECA_BIN", "${{ github.workspace }}/artifacts/bin/ddc"),
                        ("DODECA_PLUGINS", "${{ github.workspace }}/artifacts/plugins"),
                    ]),
                ]),
        );
    }

    Workflow {
        name: "CI".into(),
        on: On {
            push: Some(PushTrigger {
                tags: None,
                branches: Some(vec!["main".into()]),
            }),
            pull_request: Some(PullRequestTrigger {
                branches: Some(vec!["main".into()]),
            }),
            workflow_dispatch: None,
        },
        permissions: None,
        env: None,
        jobs,
    }
}

// =============================================================================
// Release workflow builder
// =============================================================================

/// Build the release workflow.
pub fn build_release_workflow() -> Workflow {
    use common::*;

    let mut jobs = IndexMap::new();

    // Build jobs for each target
    for target in TARGETS {
        let job_id = format!("build-{}", target.short_name());
        let archive_name = format!("dodeca-{}.{}", target.triple, target.archive_ext);

        let mut steps = vec![
            checkout(),
        ];

        // Linux ARM needs cross-compilation tools
        if target.triple == "aarch64-unknown-linux-gnu" {
            steps.push(install_rust_with_target(target.triple));
            steps.push(Step::run("Install cross-compilation tools", "sudo apt-get update && sudo apt-get install -y gcc-aarch64-linux-gnu"));
            steps.push(rust_cache());
        } else {
            steps.push(install_rust_with_target(target.triple));
            steps.push(rust_cache());
        }

        // Build WASM (uses external script)
        steps.push(Step::run("Build WASM crates", "bash scripts/build-wasm.sh").shell("bash"));

        // Build target (uses external script)
        steps.push(Step::run(
            "Build ddc and plugins",
            format!("bash scripts/build-target.sh {}", target.triple),
        ).shell("bash"));

        // Assemble archive (uses external script)
        steps.push(Step::run(
            "Assemble archive",
            format!("bash scripts/assemble-archive.sh {}", target.triple),
        ).shell("bash"));

        // Upload
        steps.push(upload_artifact(format!("build-{}", target.short_name()), archive_name));

        let mut job = Job::new(target.runner)
            .name(format!("Build ({})", target.short_name()))
            .steps(steps);

        // Windows needs special environment
        if target.triple.contains("windows") {
            job = job.env([("RUSTFLAGS", "-Ctarget-feature=+crt-static")]);
        }

        jobs.insert(job_id, job);
    }

    // Release job - creates GitHub release with all artifacts
    let build_job_ids: Vec<String> = TARGETS.iter().map(|t| format!("build-{}", t.short_name())).collect();

    jobs.insert(
        "release".into(),
        Job::new("ubuntu-latest")
            .name("Create Release")
            .needs(build_job_ids)
            .if_condition("startsWith(github.ref, 'refs/tags/')")
            .env([
                ("GH_TOKEN", "${{ secrets.GITHUB_TOKEN }}"),
                ("HOMEBREW_TAP_TOKEN", "${{ secrets.HOMEBREW_TAP_TOKEN }}"),
            ])
            .steps([
                checkout(),
                download_all_artifacts("dist"),
                Step::run("Prepare release", "bash scripts/release.sh").shell("bash"),
                Step::run("Create GitHub Release", r#"
gh release create "${{ github.ref_name }}" \
  --title "dodeca ${{ github.ref_name }}" \
  --generate-notes \
  dist/*
"#.trim()).shell("bash"),
                Step::run("Update Homebrew tap", r#"bash scripts/update-homebrew.sh "${{ github.ref_name }}""#).shell("bash"),
            ]),
    );

    Workflow {
        name: "Release".into(),
        on: On {
            push: Some(PushTrigger {
                tags: Some(vec!["v*".into()]),
                branches: None,
            }),
            // Only run on tags, not PRs - CI workflow handles PR builds
            pull_request: None,
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

const GENERATED_HEADER: &str = "# GENERATED BY: cargo xtask ci\n# DO NOT EDIT - edit xtask/src/ci.rs instead\n\n";

// =============================================================================
// Installer scripts
// =============================================================================

/// Generate the shell installer script content.
pub fn generate_installer_script() -> String {
    let repo = "bearcove/dodeca";

    format!(r##"#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://raw.githubusercontent.com/{repo}/main/install.sh | sh

set -eu

REPO="{repo}"

# Detect platform
detect_platform() {{
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64) echo "x86_64-unknown-linux-gnu" ;;
                aarch64) echo "aarch64-unknown-linux-gnu" ;;
                *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                x86_64) echo "x86_64-apple-darwin" ;;
                arm64) echo "aarch64-apple-darwin" ;;
                *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
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
    mkdir -p "$install_dir/plugins"

    # Download and extract
    local tmpdir
    tmpdir="$(mktemp -d)"
    trap "rm -rf '$tmpdir'" EXIT

    echo "Downloading..."
    curl -fsSL "$url" -o "$tmpdir/archive.tar.xz"

    echo "Extracting..."
    tar -xJf "$tmpdir/archive.tar.xz" -C "$tmpdir"

    echo "Installing..."
    cp "$tmpdir/ddc" "$install_dir/"
    chmod +x "$install_dir/ddc"

    if [ -d "$tmpdir/plugins" ]; then
        cp "$tmpdir/plugins/"* "$install_dir/plugins/"
    fi

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
"##, repo = repo)
}

/// Generate the PowerShell installer script content.
pub fn generate_powershell_installer() -> String {
    let repo = "bearcove/dodeca";

    format!(r##"# Installer for dodeca
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
"##, repo = repo)
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

/// Generate CI workflow files and installer script.
pub fn generate(repo_root: &Utf8Path, check: bool) -> Result<()> {
    let workflows_dir = repo_root.join(".github/workflows");

    // Generate CI workflow
    let ci_workflow = build_ci_workflow();
    let ci_yaml = workflow_to_yaml(&ci_workflow)?;
    check_or_write(&workflows_dir.join("ci.yml"), &ci_yaml, check)?;

    // Generate release workflow
    let release_workflow = build_release_workflow();
    let release_yaml = workflow_to_yaml(&release_workflow)?;
    check_or_write(&workflows_dir.join("release.yml"), &release_yaml, check)?;

    // Generate installer script
    let installer_content = generate_installer_script();
    check_or_write(&repo_root.join("install.sh"), &installer_content, check)?;

    Ok(())
}
