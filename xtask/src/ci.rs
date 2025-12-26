//! CI workflow generation for GitHub Actions and Forgejo Actions.
//!
//! This module provides typed representations of GitHub/Forgejo Actions workflow files
//! and generates the release workflow for dodeca.

#![allow(dead_code)] // Scaffolding for future CI features

use facet::Facet;
use indexmap::IndexMap;

// =============================================================================
// CI Platform Configuration
// =============================================================================

/// The CI platform we're generating workflows for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiPlatform {
    GitHub,
    Forgejo,
}

impl CiPlatform {
    /// Get the workflow directory path relative to repo root.
    pub fn workflows_dir(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => ".github/workflows",
            CiPlatform::Forgejo => ".forgejo/workflows",
        }
    }

    /// Get the context variable prefix (e.g., "github").
    /// Note: Forgejo uses "github" for compatibility with GitHub Actions.
    pub fn context_prefix(&self) -> &'static str {
        // Both platforms use "github" - Forgejo maintains compatibility
        "github"
    }

    /// Format a context variable reference.
    pub fn context_var(&self, var: &str) -> String {
        format!("${{{{ {}.{} }}}}", self.context_prefix(), var)
    }

    /// Get the checkout action for this platform.
    pub fn checkout_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "actions/checkout@v4",
            // Forgejo can use actions from GitHub via full URL or has its own
            CiPlatform::Forgejo => "https://github.com/actions/checkout@v4",
        }
    }

    /// Get the upload-artifact action for this platform.
    pub fn upload_artifact_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "actions/upload-artifact@v4",
            CiPlatform::Forgejo => "https://data.forgejo.org/actions/upload-artifact@v3",
        }
    }

    /// Get the download-artifact action for this platform.
    pub fn download_artifact_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "actions/download-artifact@v4",
            CiPlatform::Forgejo => "https://data.forgejo.org/actions/download-artifact@v3",
        }
    }

    /// Get the rust-toolchain action for this platform.
    /// Uses nightly for -Z checksum-freshness support (better caching).
    pub fn rust_toolchain_action(&self) -> &'static str {
        match self {
            // Note: This specifies the action, not the toolchain version.
            // The toolchain version is set via the "toolchain" input parameter.
            CiPlatform::GitHub => "dtolnay/rust-toolchain@master",
            CiPlatform::Forgejo => "https://github.com/dtolnay/rust-toolchain@master",
        }
    }

    /// The pinned nightly toolchain version to use.
    /// Using nightly for -Z checksum-freshness support (mtime-independent caching).
    pub const RUST_TOOLCHAIN: &'static str = "nightly-2025-12-19";

    /// Cargo flags for nightly features we rely on.
    /// +nightly: Explicitly use nightly toolchain (runner default may be stable).
    /// -Z checksum-freshness: Use file checksums instead of mtimes for freshness checks.
    /// -Z mtime-on-use: Update mtimes when Cargo actually uses artifacts (for sweep).
    /// This allows caching to work properly even when git checkout changes file mtimes.
    pub const CARGO_NIGHTLY_FLAGS: &'static str =
        "+nightly-2025-12-19 -Z checksum-freshness -Z mtime-on-use";

    /// Get the local cache action for this platform (for self-hosted runners).
    pub fn local_cache_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "bearcove/local-cache@a3ee51e34146df8cdfc7ea67188e9ca4e2364794",
            // Forgejo: we use ctree-based local caching (shell scripts), not an action
            CiPlatform::Forgejo => "unused",
        }
    }

    /// Check if this platform uses the local-cache action (with base path) or standard cache.
    pub fn uses_local_cache(&self) -> bool {
        matches!(self, CiPlatform::GitHub)
    }

    /// Check if this platform uses ctree for local caching (shell-based).
    pub fn uses_ctree_cache(&self) -> bool {
        matches!(self, CiPlatform::Forgejo)
    }

    /// Check if this platform uses SSH-based CAS for artifacts.
    pub fn uses_ssh_cas_artifacts(&self) -> bool {
        matches!(self, CiPlatform::Forgejo)
    }

    /// Get the Swatinem rust-cache action for this platform (for non-self-hosted runners).
    pub fn rust_cache_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "Swatinem/rust-cache@v2",
            CiPlatform::Forgejo => "https://github.com/Swatinem/rust-cache@v2",
        }
    }
}

// =============================================================================
// Configuration
// =============================================================================

/// Use self-hosted runner for macOS (true) or Depot (false).
const MACOS_SELF_HOSTED: bool = false;

/// Use self-hosted runner for Linux (true) or Depot (false).
const LINUX_SELF_HOSTED: bool = false;

/// Self-hosted runner labels for GitHub macOS.
const GITHUB_MACOS_LABELS: &[&str] = &["self-hosted", "macOS", "ARM64"];

/// Self-hosted runner labels for GitHub Linux.
const GITHUB_LINUX_LABELS: &[&str] = &["self-hosted", "Linux", "X64"];

/// Self-hosted runner labels for Forgejo macOS.
const FORGEJO_MACOS_LABELS: &[&str] = &["mac-arm64-metal"];

/// Self-hosted runner labels for Forgejo Linux.
const FORGEJO_LINUX_LABELS: &[&str] = &["linux-amd64-metal"];

/// Get target platforms for a specific CI platform.
pub fn targets_for_platform(platform: CiPlatform) -> Vec<Target> {
    let (linux_labels, macos_labels): (&[&str], &[&str]) = match platform {
        CiPlatform::GitHub => (GITHUB_LINUX_LABELS, GITHUB_MACOS_LABELS),
        CiPlatform::Forgejo => (FORGEJO_LINUX_LABELS, FORGEJO_MACOS_LABELS),
    };

    vec![
        Target {
            triple: "x86_64-unknown-linux-gnu",
            os: "ubuntu-24.04",
            runner: if LINUX_SELF_HOSTED {
                RunnerSpec::labels(linux_labels)
            } else {
                RunnerSpec::single("depot-ubuntu-24.04-32")
            },
            lib_ext: "so",
            lib_prefix: "lib",
            archive_ext: "tar.xz",
        },
        Target {
            triple: "aarch64-apple-darwin",
            os: "macos-15",
            runner: if MACOS_SELF_HOSTED {
                RunnerSpec::labels(macos_labels)
            } else {
                RunnerSpec::single("mac-arm64-oakhost")
            },
            lib_ext: "dylib",
            lib_prefix: "lib",
            archive_ext: "tar.xz",
        },
    ]
}

/// Target platforms for CI and releases (GitHub default for backwards compatibility).
pub fn default_targets() -> Vec<Target> {
    targets_for_platform(CiPlatform::GitHub)
}

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

/// Runner specification - can be a single string or a list of labels.
#[derive(Debug, Clone)]
pub enum RunnerSpec {
    /// A single runner name (e.g., "ubuntu-latest")
    Single(String),
    /// Multiple labels for self-hosted runners (e.g., ["self-hosted", "Linux", "X64"])
    Labels(Vec<String>),
}

impl RunnerSpec {
    /// Create a single runner spec from a static string.
    pub fn single(s: &str) -> Self {
        RunnerSpec::Single(s.to_string())
    }

    /// Create a labels runner spec from a slice of static strings.
    pub fn labels(labels: &[&str]) -> Self {
        RunnerSpec::Labels(labels.iter().map(|s| s.to_string()).collect())
    }

    /// Convert to the Job's runs_on field.
    pub fn to_runs_on(&self) -> RunsOn {
        match self {
            RunnerSpec::Single(s) => RunsOn::single(s.clone()),
            RunnerSpec::Labels(labels) => RunsOn::multiple(labels.iter().cloned()),
        }
    }

    /// Check if this is a self-hosted runner.
    pub fn is_self_hosted(&self) -> bool {
        matches!(self, RunnerSpec::Labels(_))
    }
}

/// A target platform configuration.
pub struct Target {
    pub triple: &'static str,
    pub os: &'static str,
    pub runner: RunnerSpec,
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

    /// Check if this target uses a self-hosted runner.
    pub fn is_self_hosted(&self) -> bool {
        self.runner.is_self_hosted()
    }

    /// Get the RunsOn configuration for this target.
    pub fn runs_on(&self) -> RunsOn {
        self.runner.to_runs_on()
    }

    /// Get the local cache base path for self-hosted runners.
    pub fn cache_base_path(&self) -> &'static str {
        match self.triple {
            "aarch64-apple-darwin" => "/Users/amos/.cache",
            "x86_64-unknown-linux-gnu" => "/home/amos/.cache",
            _ => "/tmp/.cache",
        }
    }
}

// =============================================================================
// GitHub Actions Workflow Schema
// =============================================================================

structstruck::strike! {
    /// A GitHub Actions workflow file.
    #[structstruck::each[derive(Debug, Clone, Facet)]]
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
    #[structstruck::each[derive(Debug, Clone, Facet)]]
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

/// Runner specification for GitHub Actions.
///
/// Always serialized as an array of labels, which GitHub Actions accepts for both
/// single runners (e.g., ["ubuntu-latest"]) and multi-label self-hosted runners
/// (e.g., ["self-hosted", "Linux", "X64"]).
#[derive(Debug, Clone, Facet)]
#[facet(transparent)]
pub struct RunsOn(pub Vec<String>);

impl RunsOn {
    /// Create a single runner.
    pub fn single(s: impl Into<String>) -> Self {
        RunsOn(vec![s.into()])
    }

    /// Create multiple runner labels.
    pub fn multiple(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        RunsOn(labels.into_iter().map(Into::into).collect())
    }
}

structstruck::strike! {
    /// A job in a workflow.
    #[structstruck::each[derive(Debug, Clone, Facet)]]
    #[facet(rename_all = "kebab-case")]
    pub struct Job {
        /// Display name for the job in the GitHub UI.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub name: Option<String>,

        /// The runner to use.
        pub runs_on: RunsOn,

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

        /// Permissions for this job (overrides workflow-level permissions).
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub permissions: Option<IndexMap<String, String>>,

        /// The steps to run.
        pub steps: Vec<Step>,
    }
}

structstruck::strike! {
    /// A step in a job.
    #[structstruck::each[derive(Debug, Clone, Facet)]]
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
    /// Create a new job with a single runner.
    pub fn new(runs_on: impl Into<String>) -> Self {
        Self {
            name: None,
            runs_on: RunsOn::single(runs_on),
            timeout_minutes: None,
            needs: None,
            if_condition: None,
            outputs: None,
            env: None,
            continue_on_error: None,
            permissions: None,
            steps: Vec::new(),
        }
    }

    /// Create a new job with a specific runner configuration.
    pub fn with_runner(runs_on: RunsOn) -> Self {
        Self {
            name: None,
            runs_on,
            timeout_minutes: None,
            needs: None,
            if_condition: None,
            outputs: None,
            env: None,
            continue_on_error: None,
            permissions: None,
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

    /// Set permissions for this job (overrides workflow-level permissions).
    pub fn permissions(
        mut self,
        perms: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.permissions = Some(
            perms
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
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

    pub fn checkout(platform: CiPlatform) -> Step {
        Step::uses("Checkout", platform.checkout_action())
    }

    pub fn install_rust(platform: CiPlatform) -> Step {
        Step::uses("Install Rust", platform.rust_toolchain_action())
            .with_inputs([("toolchain", CiPlatform::RUST_TOOLCHAIN)])
    }

    pub fn install_rust_with_target(platform: CiPlatform, target: &str) -> Step {
        Step::uses("Install Rust", platform.rust_toolchain_action()).with_inputs([
            ("toolchain", CiPlatform::RUST_TOOLCHAIN),
            ("targets", target),
        ])
    }

    pub fn install_rust_with_components_and_target(
        platform: CiPlatform,
        components: &str,
        target: &str,
    ) -> Step {
        Step::uses("Install Rust", platform.rust_toolchain_action()).with_inputs([
            ("toolchain", CiPlatform::RUST_TOOLCHAIN),
            ("components", components),
            ("targets", target),
        ])
    }

    pub fn rust_cache_with_targets(
        platform: CiPlatform,
        cache_targets: bool,
        target: &Target,
    ) -> Step {
        let mut inputs: Vec<(&str, &str)> = vec![
            ("cache-on-failure", "true"),
            (
                "cache-targets",
                if cache_targets { "true" } else { "false" },
            ),
        ];

        // macOS requires cache-bin: false to avoid cache corruption issues
        if target.triple.contains("apple") {
            inputs.push(("cache-bin", "false"));
        }

        Step::uses("Rust cache", platform.rust_cache_action()).with_inputs(inputs)
    }

    pub fn local_cache_with_targets(
        platform: CiPlatform,
        cache_targets: bool,
        job_suffix: &str,
        base_path: &str,
    ) -> Step {
        // Use a stable key based on OS and job type, not Cargo.lock hash.
        // This maximizes cache reuse - Cargo's incremental compilation handles
        // dependency changes well, and a stale cache is better than no cache.
        let key = if cache_targets {
            format!("${{{{ runner.os }}}}-cargo-targets-{}", job_suffix)
        } else {
            format!("${{{{ runner.os }}}}-cargo-{}", job_suffix)
        };

        let action = platform.local_cache_action();

        if platform.uses_local_cache() {
            // GitHub: use bearcove/local-cache with base path
            Step::uses("Local cache", action).with_inputs([
                ("path", "target"),
                ("key", &key),
                ("base", base_path),
            ])
        } else {
            // Forgejo: use standard cache action (no base path, different restore-keys format)
            let restore_key = "${{ runner.os }}-cargo-".to_string();
            Step::uses("Cache", action).with_inputs([
                ("path", "target"),
                ("key", &key),
                ("restore-keys", &restore_key),
            ])
        }
    }

    pub fn upload_artifact(
        platform: CiPlatform,
        name: impl Into<String>,
        path: impl Into<String>,
    ) -> Step {
        Step::uses("Upload artifact", platform.upload_artifact_action())
            .with_inputs([("name", name.into()), ("path", path.into())])
    }

    pub fn download_artifact(
        platform: CiPlatform,
        name: impl Into<String>,
        path: impl Into<String>,
    ) -> Step {
        Step::uses("Download artifact", platform.download_artifact_action())
            .with_inputs([("name", name.into()), ("path", path.into())])
    }

    pub fn download_all_artifacts(platform: CiPlatform, path: impl Into<String>) -> Step {
        Step::uses(
            "Download all artifacts",
            platform.download_artifact_action(),
        )
        .with_inputs([
            ("path", path.into()),
            ("pattern", "build-*".to_string()),
            ("merge-multiple", "true".to_string()),
        ])
    }

    // =========================================================================
    // ctree-based local cache helpers (for Forgejo self-hosted runners)
    // =========================================================================

    /// Generate a ctree-based cache restore step.
    /// Uses reflinks for near-instant COW copies on supported filesystems.
    ///
    /// After restore, we nuke any CMake build directories because CMake caches
    /// are not relocatable - they contain absolute paths that break when the
    /// workspace path changes between CI runs.
    pub fn ctree_cache_restore(cache_name: &str, base_path: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/{}", base_path, cache_name);
        Step::run(
            "Restore cache (ctree)",
            format!(
                r#"if [ -d "{cache_dir}" ]; then
  echo "Restoring cache from {cache_dir}..."
  rm -rf target 2>/dev/null || true
  ctree "{cache_dir}" target && echo "Cache restored via ctree from {cache_dir}" || echo "ctree failed, starting fresh"
  du -sh target 2>/dev/null || true
  echo "=== Cache contents after restore ==="
  tree -ah -L 2 target/ 2>/dev/null || find target -maxdepth 2 -type d

  # CMake build directories are not relocatable - they contain absolute paths.
  # Nuke them to force a fresh CMake configure on path changes.
  find target -path '*/build/*/out/build/CMakeCache.txt' -delete 2>/dev/null || true
  find target -path '*/build/*/out/build/CMakeFiles' -type d -exec rm -rf {{}} + 2>/dev/null || true
  echo "Cleaned CMake caches (non-relocatable)"

else
  echo "No cache found at {cache_dir}"
fi"#
            ),
        )
    }

    /// Restore source mtimes for unchanged files using timelord.
    pub fn timelord_restore(base_path: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/timelord", base_path);
        Step::run(
            "Restore source mtimes (timelord)",
            format!(r#"timelord sync --source-dir . --cache-dir "{cache_dir}""#),
        )
    }

    /// Print timelord cache info (if present).
    pub fn timelord_cache_info(base_path: &str, label: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/timelord", base_path);
        Step::run(
            format!("Timelord cache info ({label})"),
            format!(
                r#"if [ -d "{cache_dir}" ]; then
  timelord cache-info --cache-dir "{cache_dir}"
else
  echo "No timelord cache dir at {cache_dir}"
fi"#
            ),
        )
    }

    /// Stamp cargo sweep to track artifact usage for the cache directory.
    pub fn cargo_sweep_cache_stamp(cache_name: &str, base_path: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/{}", base_path, cache_name);
        Step::run(
            "Stamp cache artifacts (cargo sweep)",
            format!(
                r#"mkdir -p "{cache_dir}"
CARGO_TARGET_DIR="{cache_dir}" cargo sweep --stamp"#
            ),
        )
    }

    /// Trim the saved cache based on the sweep stamp.
    pub fn cargo_sweep_cache_trim(cache_name: &str, base_path: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/{}", base_path, cache_name);
        Step::run(
            "Trim cache (cargo sweep)",
            format!(
                r#"if [ -d "{cache_dir}" ]; then
  CARGO_TARGET_DIR="{cache_dir}" cargo sweep --file || true
else
  echo "No cache found at {cache_dir} to trim"
fi"#
            ),
        )
    }

    /// Generate a ctree-based cache save step.
    /// Runs at end of job to persist target/ directory.
    pub fn ctree_cache_save(cache_name: &str, base_path: &str) -> Step {
        let cache_dir = format!("{}/dodeca-ci/{}", base_path, cache_name);
        Step::run(
            "Save cache (ctree)",
            format!(
                r#"echo "=== Cache contents before save ==="
du -sh target 2>/dev/null || true
tree -ah -L 2 target/ 2>/dev/null || find target -maxdepth 2 -type d
mkdir -p "$(dirname "{cache_dir}")"
rm -rf "{cache_dir}" 2>/dev/null || true
ctree target "{cache_dir}" && echo "Cache saved via ctree to {cache_dir}" || echo "ctree save failed (non-fatal)""#
            ),
        )
    }

    // =========================================================================
    // SSH-based artifact helpers (for Forgejo) - Content-Addressed Storage
    // =========================================================================
    //
    // Artifacts are stored using content-addressed storage (CAS) over SSH for deduplication:
    // - Actual files go to: /srv/cas/cas/sha256/<hash[0:2]>/<hash>
    // - Pointer files go to: /srv/cas/cas/pointers/ci/<run_id>/<name> (contains just the hash)
    //
    // This means identical binaries across runs are only stored once.

    /// Generate an S3 artifact upload step using content-addressed storage.
    /// The file is uploaded to `cas/<hash>`, and a pointer file is written to `ci/<run_id>/<name>`.
    #[allow(dead_code)]
    pub fn s3_upload_artifact(name: &str, path: &str, run_id_var: &str) -> Step {
        Step::run(
            format!("Upload {} to S3", name),
            format!(
                r#"HASH=$(sha256sum "{path}" | cut -d' ' -f1)
CAS_KEY="cas/$HASH"
# Check if already in CAS
if ! aws s3 --endpoint-url "$S3_ENDPOINT" ls "s3://${{{{S3_BUCKET}}}}/$CAS_KEY" > /dev/null 2>&1; then
  aws s3 --endpoint-url "$S3_ENDPOINT" cp "{path}" "s3://${{{{S3_BUCKET}}}}/$CAS_KEY"
  echo "Uploaded {name} to CAS ($HASH)"
else
  echo "Skipped {name} (already in CAS: $HASH)"
fi
# Write pointer file
echo "$HASH" | aws s3 --endpoint-url "$S3_ENDPOINT" cp - "s3://${{{{S3_BUCKET}}}}/ci/{run_id}/{name}""#,
                run_id = run_id_var
            ),
        )
    }

    /// Generate an S3 artifact upload step for multiple files using content-addressed storage.
    #[allow(dead_code)]
    pub fn s3_upload_artifacts(name_prefix: &str, paths: &[String], run_id_var: &str) -> Step {
        let mut script = String::new();
        for path in paths {
            let basename = std::path::Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path);
            script.push_str(&format!(
                r#"HASH=$(sha256sum "{path}" | cut -d' ' -f1)
CAS_KEY="cas/$HASH"
if ! aws s3 --endpoint-url "$S3_ENDPOINT" ls "s3://${{S3_BUCKET}}/$CAS_KEY" > /dev/null 2>&1; then
  aws s3 --endpoint-url "$S3_ENDPOINT" cp "{path}" "s3://${{S3_BUCKET}}/$CAS_KEY"
  echo "Uploaded {basename} to CAS ($HASH)"
else
  echo "Skipped {basename} (already in CAS)"
fi
echo "$HASH" | aws s3 --endpoint-url "$S3_ENDPOINT" cp - "s3://${{S3_BUCKET}}/ci/{run_id}/{name_prefix}/{basename}"
"#,
                run_id = run_id_var
            ));
        }
        script.push_str(&format!(
            r#"echo "Processed {} files for {name_prefix}""#,
            paths.len()
        ));
        Step::run(format!("Upload {} to S3", name_prefix), script)
    }

    /// Generate an S3 artifact download step using content-addressed storage.
    /// Reads the pointer file to get the hash, then downloads from CAS.
    #[allow(dead_code)]
    pub fn s3_download_artifact(name: &str, dest_path: &str, run_id_var: &str) -> Step {
        Step::run(
            format!("Download {} from S3", name),
            format!(
                r#"mkdir -p "$(dirname "{dest_path}")"
HASH=$(aws s3 --endpoint-url "$S3_ENDPOINT" cp "s3://${{{{S3_BUCKET}}}}/ci/{run_id}/{name}" -)
aws s3 --endpoint-url "$S3_ENDPOINT" cp "s3://${{{{S3_BUCKET}}}}/cas/$HASH" "{dest_path}"
echo "Downloaded {name} from CAS ($HASH)""#,
                run_id = run_id_var
            ),
        )
    }

    /// Generate an S3 artifact download step for a prefix using content-addressed storage.
    /// Lists pointer files, reads each hash, and downloads from CAS.
    #[allow(dead_code)]
    pub fn s3_download_artifacts_prefix(prefix: &str, dest_dir: &str, run_id_var: &str) -> Step {
        Step::run(
            format!("Download {} from S3", prefix),
            format!(
                r#"mkdir -p "{dest_dir}"
# List all pointer files under the prefix
aws s3 --endpoint-url "$S3_ENDPOINT" ls "s3://${{{{S3_BUCKET}}}}/ci/{run_id}/{prefix}/" | while read -r line; do
  FILENAME=$(echo "$line" | awk '{{print $4}}')
  if [ -n "$FILENAME" ]; then
    HASH=$(aws s3 --endpoint-url "$S3_ENDPOINT" cp "s3://${{{{S3_BUCKET}}}}/ci/{run_id}/{prefix}/$FILENAME" -)
    aws s3 --endpoint-url "$S3_ENDPOINT" cp "s3://${{{{S3_BUCKET}}}}/cas/$HASH" "{dest_dir}/$FILENAME"
    echo "Downloaded $FILENAME from CAS ($HASH)"
  fi
done"#,
                run_id = run_id_var
            ),
        )
    }

    /// Generate CAS environment variables for SSH-based content-addressed storage.
    pub fn cas_env() -> IndexMap<String, String> {
        [("CAS_SSH_KEY", "${{ secrets.CAS_SSH_KEY }}")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Generate S3 environment variables for a step (legacy, for reference).
    /// Supports S3-compatible storage (Hetzner, MinIO, etc.) via S3_ENDPOINT.
    #[allow(dead_code)]
    pub fn s3_env() -> IndexMap<String, String> {
        [
            ("AWS_ACCESS_KEY_ID", "${{ secrets.S3_ACCESS_KEY }}"),
            ("AWS_SECRET_ACCESS_KEY", "${{ secrets.S3_SECRET_KEY }}"),
            ("S3_ENDPOINT", "${{ secrets.S3_ENDPOINT }}"),
            ("S3_BUCKET", "${{ secrets.S3_BUCKET }}"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    /// Generate an aws s3 command with the endpoint URL.
    pub fn s3_cmd(args: &str) -> String {
        format!(r#"aws s3 --endpoint-url "${{S3_ENDPOINT}}" {args}"#)
    }

    /// Generate a shell script to verify all expected artifacts exist.
    /// Fails CI if any binary is missing.
    pub fn verify_artifacts_script() -> String {
        let mut script = String::new();
        script.push_str("#!/bin/bash\nset -euo pipefail\n\n");
        script.push_str("echo 'Verifying all required binaries exist in dist/'\n");
        script.push_str("missing=0\n\n");

        // Check main binary
        script.push_str("if [[ ! -x dist/ddc ]]; then\n");
        script.push_str("  echo '❌ MISSING: ddc'\n");
        script.push_str("  missing=1\n");
        script.push_str("else\n");
        script.push_str("  echo '✓ ddc'\n");
        script.push_str("fi\n\n");

        // Check all cell binaries
        for (_, bin) in super::ALL_CELLS {
            script.push_str(&format!("if [[ ! -x dist/{bin} ]]; then\n"));
            script.push_str(&format!("  echo '❌ MISSING: {bin}'\n"));
            script.push_str("  missing=1\n");
            script.push_str("else\n");
            script.push_str(&format!("  echo '✓ {bin}'\n"));
            script.push_str("fi\n\n");
        }

        script.push_str("if [[ $missing -eq 1 ]]; then\n");
        script.push_str("  echo ''\n");
        script.push_str("  echo 'ERROR: Some required binaries are missing!'\n");
        script.push_str("  echo 'This usually means a cell build job failed or artifact upload was incomplete.'\n");
        script.push_str("  exit 1\n");
        script.push_str("fi\n\n");
        script.push_str("echo ''\n");
        script.push_str(&format!(
            "echo 'All {} binaries verified.'\n",
            super::ALL_CELLS.len() + 1
        ));

        script
    }
}

// =============================================================================
// CI workflow builder (for PRs and main branch)
// =============================================================================

/// CI runner configuration.
struct CiRunner {
    runner: RunnerSpec,
    wasm_install: &'static str,
}

/// Get the CI Linux runner configuration for a platform.
fn ci_linux_runner(platform: CiPlatform) -> CiRunner {
    let labels = match platform {
        CiPlatform::GitHub => GITHUB_LINUX_LABELS,
        CiPlatform::Forgejo => FORGEJO_LINUX_LABELS,
    };

    CiRunner {
        runner: if LINUX_SELF_HOSTED {
            RunnerSpec::labels(labels)
        } else {
            RunnerSpec::single("depot-ubuntu-24.04-32")
        },
        wasm_install: if LINUX_SELF_HOSTED {
            // wasm-bindgen-cli should be pre-installed on self-hosted runners
            "true"
        } else {
            // Install wasm-bindgen-cli (not wasm-pack - we call cargo directly for caching)
            "cargo install wasm-bindgen-cli --locked"
        },
    }
}

/// Build the unified CI workflow (runs on PRs, main branch, and tags).
///
/// Strategy:
/// - Fan-out: Build ddc + cell groups for each target platform
/// - Fan-in: Assemble archives after all cell groups complete
/// - Integration: Run integration tests
/// - Release: On tags, publish release (GitHub only)
pub fn build_ci_workflow(platform: CiPlatform) -> Workflow {
    use common::*;

    let ci_linux = ci_linux_runner(platform);
    let targets = targets_for_platform(platform);

    let mut jobs = IndexMap::new();
    let groups = cell_groups(9);

    // Track jobs required before release (assemble + integration per target)
    let mut all_release_needs: Vec<String> = Vec::new();

    // Cache step for CI Linux jobs
    let linux_target = targets
        .iter()
        .find(|t| t.triple == "x86_64-unknown-linux-gnu")
        .expect("Linux target should exist");
    let ci_linux_cache = if ci_linux.runner.is_self_hosted() {
        local_cache_with_targets(platform, false, "ci-linux", "/home/amos/.cache")
    } else {
        rust_cache_with_targets(platform, false, linux_target)
    };

    jobs.insert(
        "clippy".to_string(),
        Job::with_runner(ci_linux.runner.to_runs_on())
            .name("Clippy")
            .timeout(30)
            .continue_on_error(true)
            .steps([
                checkout(platform),
                install_rust_with_components_and_target(
                    platform,
                    "clippy",
                    "wasm32-unknown-unknown",
                ),
                ci_linux_cache.clone(),
                Step::run(
                    "Clippy",
                    format!(
                        "cargo {} clippy --all-features --all-targets -- -D warnings",
                        CiPlatform::CARGO_NIGHTLY_FLAGS
                    ),
                ),
            ]),
    );

    let wasm_job_id = "build-wasm".to_string();
    let wasm_artifact = "dodeca-devtools-wasm".to_string();
    jobs.insert(
        wasm_job_id.clone(),
        Job::with_runner(ci_linux.runner.to_runs_on())
            .name("Build WASM")
            .timeout(30)
            .steps([
                checkout(platform),
                install_rust_with_target(platform, "wasm32-unknown-unknown"),
                if ci_linux.runner.is_self_hosted() {
                    Step::run("Add WASM target (noop)", "true")
                } else {
                    Step::run(
                        "Add WASM target",
                        "rustup target add wasm32-unknown-unknown",
                    )
                },
                ci_linux_cache.clone(),
                Step::run("Install wasm-bindgen-cli", ci_linux.wasm_install),
                Step::run("Build WASM", "cargo xtask wasm"),
                upload_artifact(
                    platform,
                    wasm_artifact.clone(),
                    "crates/dodeca-devtools/pkg",
                ),
            ]),
    );

    for target in &targets {
        let short = target.short_name();

        // Job 1: Build ddc (main binary)
        let ddc_job_id = format!("build-ddc-{short}");
        jobs.insert(
            ddc_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Build ddc ({short})"))
                .timeout(30)
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    if target.is_self_hosted() {
                        local_cache_with_targets(
                            platform,
                            false,
                            &format!("ddc-{}", short),
                            target.cache_base_path(),
                        )
                    } else {
                        rust_cache_with_targets(platform, false, target)
                    },
                    Step::run(
                        "Build ddc",
                        format!(
                            "cargo {} build --release -p dodeca --verbose",
                            CiPlatform::CARGO_NIGHTLY_FLAGS
                        ),
                    ),
                    // Only run binary unit tests here - integration tests (serve/) need cells
                    // and run in the integration phase after assembly
                    Step::run(
                        "Test ddc",
                        format!(
                            "cargo {} test --release -p dodeca --bins",
                            CiPlatform::CARGO_NIGHTLY_FLAGS
                        ),
                    ),
                    upload_artifact(platform, format!("ddc-{short}"), "target/release/ddc"),
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
                Job::with_runner(target.runs_on())
                    .name(format!("Build cells ({short}) [{cell_names}]"))
                    .timeout(30)
                    .steps([
                        checkout(platform),
                        install_rust(platform),
                        if target.is_self_hosted() {
                            local_cache_with_targets(
                                platform,
                                true,
                                &format!("cells-{}-{}", short, group_num),
                                target.cache_base_path(),
                            )
                        } else {
                            rust_cache_with_targets(platform, true, target)
                        },
                        Step::run(
                            "Build cells",
                            format!(
                                "cargo {} build --release {build_args} --verbose",
                                CiPlatform::CARGO_NIGHTLY_FLAGS
                            ),
                        ),
                        Step::run(
                            "Test cells",
                            format!(
                                "cargo {} test --release {test_args}",
                                CiPlatform::CARGO_NIGHTLY_FLAGS
                            ),
                        ),
                        upload_artifact(
                            platform,
                            format!("cells-{short}-{group_num}"),
                            binary_paths,
                        ),
                    ]),
            );

            cell_group_jobs.push(group_job_id);
        }

        let cell_group_needs = cell_group_jobs.clone();

        // Integration tests (no wasm dependency - runs before assemble)
        let integration_job_id = format!("integration-{short}");
        let mut integration_needs = vec![ddc_job_id.clone()];
        integration_needs.extend(cell_group_needs.clone());

        // Use platform-specific workspace variable
        let workspace_var = platform.context_var("workspace");

        jobs.insert(
            integration_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Integration ({short})"))
                .timeout(30)
                .needs(integration_needs)
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    if target.is_self_hosted() {
                        local_cache_with_targets(
                            platform,
                            false,
                            &format!("integration-{}", short),
                            target.cache_base_path(),
                        )
                    } else {
                        rust_cache_with_targets(platform, false, target)
                    },
                    // Build integration-tests binary (xtask will be built in debug, so build integration-tests in debug too)
                    Step::run(
                        "Build integration-tests",
                        format!(
                            "cargo {} build -p integration-tests",
                            CiPlatform::CARGO_NIGHTLY_FLAGS
                        ),
                    ),
                    Step::uses("Download ddc", platform.download_artifact_action())
                        .with_inputs([("name", format!("ddc-{short}")), ("path", "dist".into())]),
                    Step::uses("Download cells", platform.download_artifact_action()).with_inputs(
                        [
                            ("pattern", format!("cells-{short}-*")),
                            ("path", "dist".into()),
                            ("merge-multiple", "true".into()),
                        ],
                    ),
                    // Forgejo v3 download-artifact doesn't support merge-multiple properly,
                    // so we need to flatten any subdirectories created by the pattern download.
                    Step::run(
                        "Flatten cell directories",
                        "find dist -mindepth 2 -type f -exec mv -t dist {} + 2>/dev/null || true; \
                         find dist -mindepth 1 -type d -empty -delete 2>/dev/null || true",
                    ),
                    Step::run("Prepare binaries", "chmod +x dist/ddc* && ls -la dist/"),
                    Step::run("Verify artifacts", verify_artifacts_script()),
                    Step::run(
                        "Run integration tests",
                        "cargo xtask integration --no-build",
                    )
                    .with_env([
                        ("DODECA_BIN", format!("{}/dist/ddc", workspace_var)),
                        ("DODECA_CELL_PATH", format!("{}/dist", workspace_var)),
                        (
                            "DODECA_TEST_FIXTURES_DIR",
                            format!("{}/crates/integration-tests/fixtures", workspace_var),
                        ),
                    ]),
                ]),
        );

        // Assembly job: runs after integration, downloads all artifacts and creates archive
        let assemble_job_id = format!("assemble-{short}");
        let assemble_needs = vec![integration_job_id.clone(), wasm_job_id.clone()];

        jobs.insert(
            assemble_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Assemble ({short})"))
                .timeout(30)
                .needs(assemble_needs)
                .steps([
                    checkout(platform),
                    // Download ddc binary
                    Step::uses("Download ddc", platform.download_artifact_action()).with_inputs([
                        ("name", format!("ddc-{short}")),
                        ("path", "target/release".into()),
                    ]),
                    // Download all cell group artifacts
                    Step::uses("Download cells", platform.download_artifact_action()).with_inputs([
                        ("pattern", format!("cells-{short}-*")),
                        ("path", "target/release".into()),
                        ("merge-multiple", "true".into()),
                    ]),
                    // Forgejo v3 download-artifact doesn't support merge-multiple properly
                    Step::run(
                        "Flatten cell directories",
                        "find target/release -mindepth 2 -type f -exec mv -t target/release {} + 2>/dev/null || true; \
                         find target/release -mindepth 1 -type d -empty -delete 2>/dev/null || true",
                    ),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::run(
                        "Install xz (macOS)",
                        "if command -v brew >/dev/null 2>&1; then HOMEBREW_NO_AUTO_UPDATE=1 brew install xz; fi",
                    ),
                    Step::run("List binaries", "ls -la target/release/"),
                    Step::run(
                        "Assemble archive",
                        format!("bash scripts/assemble-archive.sh {}", target.triple),
                    ),
                    upload_artifact(
                        platform,
                        format!("build-{short}"),
                        format!("dodeca-{}.{}", target.triple, target.archive_ext),
                    ),
                ]),
        );
        all_release_needs.push(assemble_job_id.clone());
    }

    // Release job (GitHub only - Forgejo uses build_forgejo_workflow)
    let ref_name_var = platform.context_var("ref_name");
    jobs.insert(
        "release".into(),
        Job::new("ubuntu-latest")
            .name("Release")
            .timeout(30)
            .needs(all_release_needs)
            .if_condition("startsWith(github.ref, 'refs/tags/')")
            .permissions([("contents", "write")])
            .env([
                ("GH_TOKEN", "${{ secrets.GITHUB_TOKEN }}"),
                ("HOMEBREW_TAP_TOKEN", "${{ secrets.HOMEBREW_TAP_TOKEN }}"),
            ])
            .steps([
                checkout(platform),
                download_all_artifacts(platform, "dist"),
                Step::run("List artifacts (before flatten)", "ls -laR dist/"),
                Step::run(
                    "Flatten artifact directories",
                    "find dist -mindepth 2 -type f -exec mv -t dist {} + 2>/dev/null || true; find dist -mindepth 1 -type d -empty -delete 2>/dev/null || true",
                ),
                Step::run("List artifacts (after flatten)", "ls -la dist/"),
                Step::run(
                    "Create GitHub Release",
                    format!(
                        r#"shopt -s nullglob
# Rename install.sh to dodeca-installer.sh for the release
cp install.sh dist/dodeca-installer.sh
gh release create "{ref_name}" \
  --title "dodeca {ref_name}" \
  --generate-notes \
  dist/*.tar.xz dist/*.zip dist/dodeca-installer.sh"#,
                        ref_name = ref_name_var
                    ),
                )
                .shell("bash"),
                Step::run(
                    "Update Homebrew tap",
                    format!(
                        r#"bash scripts/update-homebrew.sh "{ref_name}""#,
                        ref_name = ref_name_var
                    ),
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
            [("contents", "read")]
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
// Forgejo CI workflow builder (ctree cache + SSH CAS artifacts)
// =============================================================================

/// Build the Forgejo CI workflow with local ctree caching and SSH-based CAS artifacts.
///
/// This is completely separate from the GitHub workflow because:
/// - Uses ctree for near-instant COW cache copies on local filesystems
/// - Uses SSH + rsync for artifact storage (content-addressed, deduplicated)
/// - Simpler structure optimized for self-hosted runners
pub fn build_forgejo_workflow() -> Workflow {
    use common::*;

    let platform = CiPlatform::Forgejo;
    let targets = targets_for_platform(platform);
    let linux_cache_base = targets
        .iter()
        .find(|target| target.triple == "x86_64-unknown-linux-gnu")
        .map(|target| target.cache_base_path())
        .unwrap_or("/home/amos/.cache");
    let groups = cell_groups(9);

    // CAS env vars used by all artifact steps (SSH-based content-addressed storage)
    let cas_env = cas_env();

    // Run ID for S3 artifact paths
    let run_id = "${{ github.run_id }}";

    let mut jobs = IndexMap::new();
    let mut all_release_needs: Vec<String> = Vec::new();

    // -------------------------------------------------------------------------
    // Clippy job
    // -------------------------------------------------------------------------
    jobs.insert(
        "clippy".to_string(),
        Job::with_runner(RunnerSpec::labels(FORGEJO_LINUX_LABELS).to_runs_on())
            .name("Clippy")
            .timeout(30)
            .continue_on_error(true)
            .steps([
                checkout(platform),
                install_rust_with_components_and_target(
                    platform,
                    "clippy",
                    "wasm32-unknown-unknown",
                ),
                ctree_cache_restore("clippy", linux_cache_base),
                timelord_cache_info(linux_cache_base, "before sync"),
                timelord_restore(linux_cache_base),
                cargo_sweep_cache_stamp("clippy", linux_cache_base),
                Step::run(
                    "Clippy",
                    format!(
                        "cargo {} clippy --all-features --all-targets -- -D warnings",
                        CiPlatform::CARGO_NIGHTLY_FLAGS
                    ),
                ),
                ctree_cache_save("clippy", linux_cache_base),
                cargo_sweep_cache_trim("clippy", linux_cache_base),
            ]),
    );

    // -------------------------------------------------------------------------
    // Build WASM job
    // -------------------------------------------------------------------------
    let wasm_job_id = "build-wasm".to_string();
    jobs.insert(
        wasm_job_id.clone(),
        Job::with_runner(RunnerSpec::labels(FORGEJO_LINUX_LABELS).to_runs_on())
            .name("Build WASM")
            .timeout(30)
            .env(cas_env.clone())
            .steps([
                checkout(platform),
                install_rust_with_target(platform, "wasm32-unknown-unknown"),
                ctree_cache_restore("wasm", linux_cache_base),
                timelord_cache_info(linux_cache_base, "before sync"),
                timelord_restore(linux_cache_base),
                cargo_sweep_cache_stamp("wasm", linux_cache_base),
                Step::run("Build WASM", "cargo xtask wasm"),
                // Save cache immediately after build (before uploads that might fail)
                ctree_cache_save("wasm", linux_cache_base),
                cargo_sweep_cache_trim("wasm", linux_cache_base),
                // Upload WASM to CAS as a tarball
                Step::run(
                    "Upload WASM to CAS",
                    format!(
                        r#"tar -czf /tmp/wasm.tar.gz -C crates/dodeca-devtools pkg
./scripts/cas-upload.ts /tmp/wasm.tar.gz "ci/{run_id}/wasm.tar.gz""#
                    ),
                ),
            ]),
    );

    // -------------------------------------------------------------------------
    // Per-target jobs
    // -------------------------------------------------------------------------
    for target in &targets {
        let short = target.short_name();
        let cache_base = target.cache_base_path();

        // Build ddc (main binary)
        let ddc_job_id = format!("build-ddc-{short}");
        jobs.insert(
            ddc_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Build ddc ({short})"))
                .timeout(30)
                .env(cas_env.clone())
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    ctree_cache_restore(&format!("ddc-{short}"), cache_base),
                    timelord_cache_info(cache_base, "before sync"),
                    timelord_restore(cache_base),
                    cargo_sweep_cache_stamp(&format!("ddc-{short}"), cache_base),
                    Step::run(
                        "Debug proc-macro2 mtimes",
                        r#"set -euo pipefail
if ls target/debug/build/proc-macro2-*/build-script-build >/dev/null 2>&1; then
  if stat -c %y target/debug/build/proc-macro2-*/build-script-build >/dev/null 2>&1; then
    stat -c %y target/debug/build/proc-macro2-*/build-script-build
  else
    stat -f %m target/debug/build/proc-macro2-*/build-script-build
  fi
else
  echo "No proc-macro2 build-script-build file found"
fi"#,
                    )
                    .shell("bash"),
                    Step::run("Build ddc", format!("cargo {} build --release -p dodeca --verbose", CiPlatform::CARGO_NIGHTLY_FLAGS)),
                    // Save cache immediately after build (before tests/uploads that might fail)
                    ctree_cache_save(&format!("ddc-{short}"), cache_base),
                    cargo_sweep_cache_trim(&format!("ddc-{short}"), cache_base),
                    Step::run("Test ddc", format!("cargo {} test --release -p dodeca --bins", CiPlatform::CARGO_NIGHTLY_FLAGS)),
                    Step::run(
                        "Upload ddc to CAS",
                        format!(
                            r#"./scripts/cas-upload.ts target/release/ddc "ci/{run_id}/ddc-{short}""#
                        ),
                    ),
                ]),
        );

        // Build cell groups in parallel
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

            // Upload cell binaries to CAS with pointer files (batch upload)
            // Each group gets its own manifest to avoid overwriting
            let binary_list: String = cells
                .iter()
                .map(|(_, bin)| format!("target/release/{bin}"))
                .collect::<Vec<_>>()
                .join(" ");
            let upload_script = format!(
                r#"./scripts/cas-upload-batch.ts "ci/{run_id}/cells-{short}-{group_num}" {binary_list}"#
            );

            jobs.insert(
                group_job_id.clone(),
                Job::with_runner(target.runs_on())
                    .name(format!("Build cells ({short}) [{cell_names}]"))
                    .timeout(30)
                    .env(cas_env.clone())
                    .steps([
                        checkout(platform),
                        install_rust(platform),
                        ctree_cache_restore(&format!("cells-{short}-{group_num}"), cache_base),
                        timelord_cache_info(cache_base, "before sync"),
                        timelord_restore(cache_base),
                        cargo_sweep_cache_stamp(&format!("cells-{short}-{group_num}"), cache_base),
                        Step::run(
                            "Build cells",
                            format!(
                                "cargo {} build --release {build_args} --verbose",
                                CiPlatform::CARGO_NIGHTLY_FLAGS
                            ),
                        ),
                        // Save cache immediately after build (before tests/uploads that might fail)
                        ctree_cache_save(&format!("cells-{short}-{group_num}"), cache_base),
                        cargo_sweep_cache_trim(&format!("cells-{short}-{group_num}"), cache_base),
                        Step::run(
                            "Test cells",
                            format!(
                                "cargo {} test --release {test_args}",
                                CiPlatform::CARGO_NIGHTLY_FLAGS
                            ),
                        ),
                        Step::run("Upload cells to CAS", upload_script),
                    ]),
            );

            cell_group_jobs.push(group_job_id);
        }

        // Integration tests
        let integration_job_id = format!("integration-{short}");
        let mut integration_needs = vec![ddc_job_id.clone()];
        integration_needs.extend(cell_group_jobs.clone());

        let workspace_var = platform.context_var("workspace");

        jobs.insert(
            integration_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Integration ({short})"))
                .timeout(30)
                .needs(integration_needs)
                .env(cas_env.clone())
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    ctree_cache_restore(&format!("integration-{short}"), cache_base),
                    timelord_cache_info(cache_base, "before sync"),
                    timelord_restore(cache_base),
                    cargo_sweep_cache_stamp(&format!("integration-{short}"), cache_base),
                    // Build integration-tests binary (xtask will be built in debug, so build integration-tests in debug too)
                    Step::run(
                        "Build integration-tests",
                        format!(
                            "cargo {} build -p integration-tests",
                            CiPlatform::CARGO_NIGHTLY_FLAGS
                        ),
                    ),
                    // Download ddc from CAS
                    Step::run(
                        "Download ddc from CAS",
                        format!(r#"./scripts/cas-download.ts "ci/{run_id}/ddc-{short}" dist/ddc"#),
                    ),
                    // Download cells from CAS - download all group manifests
                    Step::run(
                        "Download cells from CAS",
                        {
                            let download_commands: Vec<String> = groups.iter()
                                .map(|(group_num, _)| {
                                    format!(r#"./scripts/cas-download-batch.ts "ci/{run_id}/cells-{short}-{group_num}" dist/"#)
                                })
                                .collect();
                            download_commands.join("\n")
                        },
                    ),
                    Step::run("List binaries", "ls -la dist/"),
                    Step::run("Verify artifacts", verify_artifacts_script()),
                    Step::run(
                        "Run integration tests",
                        "cargo xtask integration --no-build",
                    )
                    .with_env([
                        ("DODECA_BIN", format!("{}/dist/ddc", workspace_var)),
                        ("DODECA_CELL_PATH", format!("{}/dist", workspace_var)),
                        ("DODECA_TEST_FIXTURES_DIR", format!("{}/crates/integration-tests/fixtures", workspace_var)),
                    ]),
                    ctree_cache_save(&format!("integration-{short}"), cache_base),
                    cargo_sweep_cache_trim(&format!("integration-{short}"), cache_base),
                ]),
        );

        // Assemble job
        let assemble_job_id = format!("assemble-{short}");
        let assemble_needs = vec![integration_job_id.clone(), wasm_job_id.clone()];

        jobs.insert(
            assemble_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Assemble ({short})"))
                .timeout(30)
                .needs(assemble_needs)
                .env(cas_env.clone())
                .steps([
                    checkout(platform),
                    // Download ddc from CAS
                    Step::run(
                        "Download ddc from CAS",
                        format!(
                            r#"./scripts/cas-download.ts "ci/{run_id}/ddc-{short}" target/release/ddc"#
                        ),
                    ),
                    // Download cells from CAS - download all group manifests
                    Step::run(
                        "Download cells from CAS",
                        {
                            let download_commands: Vec<String> = groups.iter()
                                .map(|(group_num, _)| {
                                    format!(r#"./scripts/cas-download-batch.ts "ci/{run_id}/cells-{short}-{group_num}" target/release/"#)
                                })
                                .collect();
                            download_commands.join("\n")
                        },
                    ),
                    // Download WASM from CAS
                    Step::run(
                        "Download WASM from CAS",
                        format!(
                            r#"./scripts/cas-download.ts "ci/{run_id}/wasm.tar.gz" /tmp/wasm.tar.gz
mkdir -p crates/dodeca-devtools
tar -xzf /tmp/wasm.tar.gz -C crates/dodeca-devtools"#
                        ),
                    ),
                    Step::run(
                        "Install xz (macOS)",
                        "if command -v brew >/dev/null 2>&1; then HOMEBREW_NO_AUTO_UPDATE=1 brew install xz; fi",
                    ),
                    Step::run("List binaries", "ls -la target/release/"),
                    Step::run(
                        "Assemble archive",
                        format!("bash scripts/assemble-archive.sh {}", target.triple),
                    ),
                    // Upload final archive to CAS
                    Step::run(
                        "Upload archive to CAS",
                        format!(
                            r#"./scripts/cas-upload.ts "dodeca-{triple}.{ext}" "ci/{run_id}/dodeca-{triple}.{ext}""#,
                            triple = target.triple,
                            ext = target.archive_ext
                        ),
                    ),
                ]),
        );

        all_release_needs.push(assemble_job_id);
    }

    // -------------------------------------------------------------------------
    // Release job
    // -------------------------------------------------------------------------
    let ref_name_var = platform.context_var("ref_name");
    jobs.insert(
        "release".into(),
        Job::with_runner(RunnerSpec::labels(FORGEJO_LINUX_LABELS).to_runs_on())
            .name("Release")
            .timeout(30)
            .needs(all_release_needs)
            .if_condition("startsWith(github.ref, 'refs/tags/')")
            .env(cas_env)
            .steps([
                checkout(platform),
                // Download all archives from CAS
                Step::run(
                    "Download archives from CAS",
                    format!(
                        r#"mkdir -p dist
./scripts/cas-download.ts "ci/{run_id}/dodeca-x86_64-unknown-linux-gnu.tar.xz" dist/dodeca-x86_64-unknown-linux-gnu.tar.xz
./scripts/cas-download.ts "ci/{run_id}/dodeca-aarch64-apple-darwin.tar.xz" dist/dodeca-aarch64-apple-darwin.tar.xz
ls -la dist/"#
                    ),
                ),
                Step::run(
                    "Show release info",
                    format!(r#"echo "Release: {ref_name}"; ls -la dist/"#, ref_name = ref_name_var),
                ),
                // TODO: Add Forgejo release API call here if needed
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
            [("contents", "read")]
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

/// Generate GitHub Actions workflow and installer script.
pub fn generate_github(repo_root: &Utf8Path, check: bool) -> Result<()> {
    // Generate GitHub Actions workflow
    let github_workflows_dir = repo_root.join(CiPlatform::GitHub.workflows_dir());
    let github_ci_workflow = build_ci_workflow(CiPlatform::GitHub);
    let github_ci_yaml = workflow_to_yaml(&github_ci_workflow)?;
    check_or_write(&github_workflows_dir.join("ci.yml"), &github_ci_yaml, check)?;

    // Generate installer script (for GitHub releases)
    let installer_content = generate_installer_script();
    check_or_write(&repo_root.join("install.sh"), &installer_content, check)?;

    Ok(())
}

/// Generate Forgejo Actions workflow.
pub fn generate_forgejo(repo_root: &Utf8Path, check: bool) -> Result<()> {
    // Generate Forgejo Actions workflow (completely separate implementation)
    let forgejo_workflows_dir = repo_root.join(CiPlatform::Forgejo.workflows_dir());
    let forgejo_ci_workflow = build_forgejo_workflow();
    let forgejo_ci_yaml = workflow_to_yaml(&forgejo_ci_workflow)?;
    check_or_write(
        &forgejo_workflows_dir.join("ci.yml"),
        &forgejo_ci_yaml,
        check,
    )?;

    Ok(())
}
