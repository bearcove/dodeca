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

    /// The pinned toolchain to use across all CI jobs.
    pub const RUST_TOOLCHAIN: &'static str = "stable";

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

    /// Get the wasm-pack installer action for this platform.
    pub fn wasm_pack_action(&self) -> &'static str {
        match self {
            CiPlatform::GitHub => "jetli/wasm-pack-action@v0.4.0",
            CiPlatform::Forgejo => "https://github.com/jetli/wasm-pack-action@v0.4.0",
        }
    }
}

// =============================================================================
// Configuration
// =============================================================================

/// Bearcove-hosted Linux runner for compile-heavy jobs.
const GITHUB_LINUX_RUNNER: &str = "bearcove-ubuntu-24.04";

/// Free GitHub-hosted macOS runner for every-commit compile checks.
const GITHUB_MACOS_CHECK_RUNNER: &str = "macos-15";

/// Free GitHub-hosted Windows runner for every-commit compile checks.
const GITHUB_WINDOWS_CHECK_RUNNER: &str = "windows-latest";

/// Blacksmith macOS runner for tag-only release packaging (bigger = binaries ready sooner).
const GITHUB_MACOS_PACKAGE_RUNNER: &str = "blacksmith-12vcpu-macos-15";

/// Blacksmith Windows runner for tag-only release packaging.
const GITHUB_WINDOWS_PACKAGE_RUNNER: &str = "blacksmith-8vcpu-windows-2025";

/// Self-hosted runner labels for Forgejo macOS.
const FORGEJO_MACOS_LABELS: &[&str] = &["mac"];

/// Self-hosted runner labels for Forgejo Linux.
const FORGEJO_LINUX_LABELS: &[&str] = &["linux-x86_64-trusted"];

/// Get target platforms for a specific CI platform.
pub fn targets_for_platform(platform: CiPlatform) -> Vec<Target> {
    vec![
        Target {
            triple: "x86_64-unknown-linux-gnu",
            os: "ubuntu-24.04",
            runner: linux_runner(platform),
            archive_ext: "tar.xz",
        },
        Target {
            triple: "aarch64-apple-darwin",
            os: "macos-15",
            runner: macos_runner(platform),
            archive_ext: "tar.xz",
        },
    ]
}

/// Target platforms for CI and releases (GitHub default for backwards compatibility).
pub fn default_targets() -> Vec<Target> {
    targets_for_platform(CiPlatform::GitHub)
}

/// Get the Linux runner for a CI platform.
fn linux_runner(platform: CiPlatform) -> RunnerSpec {
    match platform {
        CiPlatform::GitHub => RunnerSpec::single(GITHUB_LINUX_RUNNER),
        CiPlatform::Forgejo => RunnerSpec::labels(FORGEJO_LINUX_LABELS),
    }
}

/// Get the macOS runner for a CI platform.
fn macos_runner(platform: CiPlatform) -> RunnerSpec {
    match platform {
        CiPlatform::GitHub => RunnerSpec::single(GITHUB_MACOS_CHECK_RUNNER),
        CiPlatform::Forgejo => RunnerSpec::labels(FORGEJO_MACOS_LABELS),
    }
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

        /// Trigger on merge group events (for merge queues).
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub merge_group: Option<pub struct MergeGroupTrigger {
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

#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "kebab-case")]
pub struct Container {
    pub image: String,
    #[facet(default, skip_serializing_if = Option::is_none)]
    pub volumes: Option<Vec<String>>,
    #[facet(default, skip_serializing_if = Option::is_none)]
    pub options: Option<String>,
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

        /// Optional job container.
        #[facet(default, skip_serializing_if = Option::is_none)]
        pub container: Option<Container>,

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
            container: None,
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
            container: None,
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

    /// Run this job inside a container.
    pub fn container(
        mut self,
        image: impl Into<String>,
        volumes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.container = Some(Container {
            image: image.into(),
            volumes: Some(volumes.into_iter().map(Into::into).collect()),
            options: None,
        });
        self
    }

    /// Set container runtime options.
    pub fn container_options(mut self, options: impl Into<String>) -> Self {
        let container = self
            .container
            .as_mut()
            .expect("container_options called before container");
        container.options = Some(options.into());
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

const BUILD_DDC_COMMAND: &str = r#"set -euo pipefail
if [[ "${GITHUB_REF_TYPE:-}" == "tag" && -n "${GITHUB_REF_NAME:-}" ]]; then
  export DODECA_RELEASE_VERSION="${GITHUB_REF_NAME}"
fi
cargo build --release --bin ddc --verbose
actual="$(target/release/ddc --version)"
echo "$actual"
if [[ "${GITHUB_REF_TYPE:-}" == "tag" && -n "${GITHUB_REF_NAME:-}" ]]; then
  expected="ddc ${GITHUB_REF_NAME#v}"
  if [[ "$actual" != "$expected" ]]; then
    echo "Expected '$expected', got '$actual'" >&2
    exit 1
  fi
fi"#;

const TEST_DDC_COMMAND: &str = r#"set -euo pipefail
if [[ "${GITHUB_REF_TYPE:-}" == "tag" && -n "${GITHUB_REF_NAME:-}" ]]; then
  export DODECA_RELEASE_VERSION="${GITHUB_REF_NAME}"
fi
cargo test --release --bin ddc"#;

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

    /// Generate a shell script to verify the required release artifacts exist.
    pub fn verify_artifacts_script() -> String {
        let mut script = String::new();
        script.push_str("#!/bin/bash\nset -euo pipefail\n\n");
        script.push_str("echo 'Verifying all required artifacts exist in dist/'\n");
        script.push_str("missing=0\n\n");

        // Check main binary
        script.push_str("if [[ ! -x dist/ddc ]]; then\n");
        script.push_str("  echo '❌ MISSING: ddc'\n");
        script.push_str("  missing=1\n");
        script.push_str("else\n");
        script.push_str("  echo '✓ ddc'\n");
        script.push_str("fi\n\n");

        script.push_str("if [[ $missing -eq 1 ]]; then\n");
        script.push_str("  echo ''\n");
        script.push_str("  echo 'ERROR: Some required binaries are missing!'\n");
        script.push_str("  echo 'This usually means a ddc build job failed or artifact upload was incomplete.'\n");
        script.push_str("  exit 1\n");
        script.push_str("fi\n\n");
        script.push_str("echo ''\n");
        script.push_str("echo 'All artifacts verified.'\n");

        script
    }
}

// =============================================================================
// CI workflow builder (for PRs and main branch)
// =============================================================================

/// CI runner configuration.
struct CiRunner {
    runner: RunnerSpec,
}

/// Get the CI Linux runner configuration for a platform.
fn ci_linux_runner(platform: CiPlatform) -> CiRunner {
    let runner = linux_runner(platform);

    CiRunner { runner }
}

/// Build the unified CI workflow (runs on PRs, main branch, and tags).
///
/// Strategy:
/// - Fan-out: Build ddc for each target platform
/// - Integration: Run integration tests
/// - Release: On tags, publish release (GitHub only)
pub fn build_ci_workflow(platform: CiPlatform, _repo_root: &Utf8Path) -> Workflow {
    use common::*;

    let ci_linux = ci_linux_runner(platform);
    let targets = targets_for_platform(platform);

    let mut jobs = IndexMap::new();

    // Track jobs required before release.
    let mut all_release_needs: Vec<String> = Vec::new();

    // Verify generated CI files are up to date (no dependencies, runs in parallel with everything)
    jobs.insert(
        "check-ci".to_string(),
        Job::with_runner(ci_linux_runner(platform).runner.to_runs_on())
            .name("Check CI up to date")
            .timeout(10)
            .steps([
                checkout(platform),
                install_rust(platform),
                if ci_linux_runner(platform).runner.is_self_hosted() {
                    local_cache_with_targets(platform, false, "check-ci", "/home/amos/.cache")
                } else {
                    rust_cache_with_targets(
                        platform,
                        false,
                        targets
                            .iter()
                            .find(|t| t.triple == "x86_64-unknown-linux-gnu")
                            .expect("Linux target should exist"),
                    )
                },
                Step::run(
                    "Check CI files are up to date",
                    "cargo xtask ci-github --check && cargo xtask ci-forgejo --check",
                ),
            ]),
    );

    // Cache step for CI Linux jobs
    let linux_target = targets
        .iter()
        .find(|t| t.triple == "x86_64-unknown-linux-gnu")
        .expect("Linux target should exist");
    let ci_linux_cache = if ci_linux.runner.is_self_hosted() {
        local_cache_with_targets(platform, true, "ci-linux", "/home/amos/.cache")
    } else {
        rust_cache_with_targets(platform, true, linux_target)
    };

    // Build the WASM bundles that dodeca embeds at compile time via include_bytes!.
    let wasm_job_id = "build-wasm".to_string();
    let devtools_wasm_artifact = "dodeca-devtools-wasm".to_string();
    let search_wasm_artifact = "dodeca-search-wasm".to_string();
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
                Step::uses("Install wasm-pack", platform.wasm_pack_action())
                    .with_inputs([("version", "latest")]),
                Step::run(
                    "Build embedded WASM",
                    r#"wasm-pack build crates/dodeca-devtools --target web --target-dir target/wasm-pack
wasm-pack build crates/dodeca-search-wasm --target web --target-dir target/wasm-pack"#,
                ),
                upload_artifact(
                    platform,
                    devtools_wasm_artifact.clone(),
                    "crates/dodeca-devtools/pkg",
                ),
                upload_artifact(
                    platform,
                    search_wasm_artifact.clone(),
                    "crates/dodeca-search-wasm/pkg",
                ),
            ]),
    );

    // Clippy depends on WASM because dodeca embeds WASM bundles at compile time.
    jobs.insert(
        "clippy".to_string(),
        Job::with_runner(ci_linux.runner.to_runs_on())
            .name("Clippy")
            .timeout(30)
            .continue_on_error(true)
            .needs([wasm_job_id.clone()])
            .steps([
                checkout(platform),
                install_rust_with_components_and_target(
                    platform,
                    "clippy",
                    "wasm32-unknown-unknown",
                ),
                ci_linux_cache.clone(),
                // Download WASM bundles embedded at compile time via include_bytes!.
                Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                    ("name", devtools_wasm_artifact.clone()),
                    ("path", "crates/dodeca-devtools/pkg".into()),
                ]),
                Step::uses("Download search WASM", platform.download_artifact_action())
                    .with_inputs([
                        ("name", search_wasm_artifact.clone()),
                        ("path", "crates/dodeca-search-wasm/pkg".into()),
                    ]),
                Step::run(
                    "Clippy",
                    "cargo clippy --all-features --all-targets -- -D warnings",
                ),
            ]),
    );

    // Every-commit compile checks on free GitHub-hosted macOS/Windows runners.
    // Compile signal only, no packaging. dodeca embeds WASM via include_bytes!,
    // so these depend on the Linux-built WASM bundles.
    if platform == CiPlatform::GitHub {
        jobs.insert(
            "check-macos".to_string(),
            Job::with_runner(RunnerSpec::single(GITHUB_MACOS_CHECK_RUNNER).to_runs_on())
                .name("Check macOS")
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Check macOS", "cargo check --workspace --all-targets"),
                ]),
        );

        jobs.insert(
            "check-windows".to_string(),
            Job::with_runner(RunnerSpec::single(GITHUB_WINDOWS_CHECK_RUNNER).to_runs_on())
                .name("Check Windows")
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Check Windows", "cargo check --workspace --all-targets"),
                ]),
        );
    }

    // Linux carries the real validation on every commit: build ddc (needed by
    // integration + browser tests), integration tests, and browser tests.
    for target in &targets {
        if target.triple != "x86_64-unknown-linux-gnu" {
            continue;
        }
        let short = target.short_name();
        let workspace_var = platform.context_var("workspace");

        // Build ddc for integration + browser tests. No archive/upload here;
        // release archives are produced by tag-only package jobs.
        let ddc_job_id = format!("build-ddc-{short}");
        jobs.insert(
            ddc_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Build ddc ({short})"))
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    if target.is_self_hosted() {
                        local_cache_with_targets(
                            platform,
                            true,
                            &format!("ddc-{}", short),
                            target.cache_base_path(),
                        )
                    } else {
                        rust_cache_with_targets(platform, true, target)
                    },
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Build ddc", BUILD_DDC_COMMAND).shell("bash"),
                    Step::run("Test ddc", TEST_DDC_COMMAND).shell("bash"),
                    upload_artifact(platform, format!("ddc-{short}"), "target/release/ddc"),
                ]),
        );

        let integration_job_id = format!("integration-{short}");
        jobs.insert(
            integration_job_id.clone(),
            Job::with_runner(target.runs_on())
                .name(format!("Integration ({short})"))
                .timeout(30)
                .needs([ddc_job_id.clone(), wasm_job_id.clone()])
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    if target.is_self_hosted() {
                        local_cache_with_targets(
                            platform,
                            true,
                            &format!("integration-{}", short),
                            target.cache_base_path(),
                        )
                    } else {
                        rust_cache_with_targets(platform, true, target)
                    },
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run(
                        "Build integration-tests",
                        "cargo build --package integration-tests",
                    ),
                    Step::uses("Download ddc", platform.download_artifact_action())
                        .with_inputs([("name", format!("ddc-{short}")), ("path", "dist".into())]),
                    Step::run("Prepare artifacts", "chmod +x dist/ddc && ls -la dist/"),
                    Step::run("Verify artifacts", verify_artifacts_script()),
                    Step::run(
                        "Run integration tests",
                        "cargo xtask integration --no-build",
                    )
                    .with_env([
                        ("DODECA_BIN", format!("{}/dist/ddc", workspace_var)),
                        (
                            "DODECA_TEST_FIXTURES_DIR",
                            format!("{}/crates/integration-tests/fixtures", workspace_var),
                        ),
                    ]),
                ]),
        );
        all_release_needs.push(integration_job_id.clone());

        // Browser tests (Linux only) - tests livereload and DOM patching in real browser
        let browser_tests_job_id = format!("browser-tests-{short}");
        jobs.insert(
            browser_tests_job_id,
            Job::with_runner(target.runs_on())
                .name(format!("Browser Tests ({short})"))
                .timeout(30)
                .needs([ddc_job_id.clone(), wasm_job_id.clone()])
                .steps([
                    checkout(platform),
                    Step::uses("Download ddc", platform.download_artifact_action())
                        .with_inputs([("name", format!("ddc-{short}")), ("path", "dist".into())]),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::run("Prepare artifacts", "chmod +x dist/ddc && ls -la dist/"),
                    Step::uses("Setup Node.js", "actions/setup-node@v4")
                        .with_inputs([("node-version", "20")]),
                    Step::run(
                        "Install browser test dependencies",
                        "cd browser-tests && npm ci && npx playwright install chromium --with-deps",
                    ),
                    Step::run("Run browser tests", "cd browser-tests && npm test")
                        .with_env([("DODECA_BIN", format!("{}/dist/ddc", workspace_var))]),
                ]),
        );
    }

    // Tag-only packaging. Binaries should be ready soon, so macOS uses the bigger
    // Blacksmith runner; Linux stays on the bearcove pool (fixed cost, already fast).
    if platform == CiPlatform::GitHub {
        let linux_pkg = "package-linux-x64".to_string();
        jobs.insert(
            linux_pkg.clone(),
            Job::with_runner(linux_runner(platform).to_runs_on())
                .name("Package (linux-x64)")
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .if_condition("startsWith(github.ref, 'refs/tags/')")
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    rust_cache_with_targets(platform, true, linux_target),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Build ddc", BUILD_DDC_COMMAND).shell("bash"),
                    Step::run(
                        "Assemble archive",
                        "bash scripts/assemble-archive.sh x86_64-unknown-linux-gnu",
                    ),
                    upload_artifact(
                        platform,
                        "build-linux-x64",
                        "dodeca-x86_64-unknown-linux-gnu.tar.xz",
                    ),
                ]),
        );
        all_release_needs.push(linux_pkg);

        let mac_pkg = "package-macos-arm64".to_string();
        jobs.insert(
            mac_pkg.clone(),
            Job::with_runner(RunnerSpec::single(GITHUB_MACOS_PACKAGE_RUNNER).to_runs_on())
                .name("Package (macos-arm64)")
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .if_condition("startsWith(github.ref, 'refs/tags/')")
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Build ddc", BUILD_DDC_COMMAND).shell("bash"),
                    Step::run(
                        "Install xz (macOS)",
                        "if ! command -v xz >/dev/null 2>&1 && command -v brew >/dev/null 2>&1; then HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1 brew install xz; fi",
                    ),
                    Step::run(
                        "Assemble archive",
                        "bash scripts/assemble-archive.sh aarch64-apple-darwin",
                    ),
                    upload_artifact(
                        platform,
                        "build-macos-arm64",
                        "dodeca-aarch64-apple-darwin.tar.xz",
                    ),
                ]),
        );
        all_release_needs.push(mac_pkg);

        let win_pkg = "package-windows-x64".to_string();
        jobs.insert(
            win_pkg.clone(),
            Job::with_runner(RunnerSpec::single(GITHUB_WINDOWS_PACKAGE_RUNNER).to_runs_on())
                .name("Package (windows-x64)")
                .timeout(30)
                .needs([wasm_job_id.clone()])
                .if_condition("startsWith(github.ref, 'refs/tags/')")
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    Step::uses("Download WASM", platform.download_artifact_action()).with_inputs([
                        ("name", devtools_wasm_artifact.clone()),
                        ("path", "crates/dodeca-devtools/pkg".into()),
                    ]),
                    Step::uses("Download search WASM", platform.download_artifact_action())
                        .with_inputs([
                            ("name", search_wasm_artifact.clone()),
                            ("path", "crates/dodeca-search-wasm/pkg".into()),
                        ]),
                    Step::run("Build ddc", BUILD_DDC_COMMAND).shell("bash"),
                    Step::run(
                        "Assemble archive",
                        "bash scripts/assemble-archive.sh x86_64-pc-windows-msvc",
                    ),
                    upload_artifact(
                        platform,
                        "build-windows-x64",
                        "dodeca-x86_64-pc-windows-msvc.zip",
                    ),
                ]),
        );
        all_release_needs.push(win_pkg);
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
            merge_group: Some(MergeGroupTrigger {
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
// Forgejo CI workflow builder (two fat vixen-ci jobs)
// =============================================================================

/// Build the Forgejo CI workflow.
///
/// Forgejo has one trusted Linux runner and one macOS runner. Keep the workflow
/// coarse: each platform builds, tests, assembles, and uploads only its final
/// archive. Linux uses the shared vixen-ci image and mounted cache; macOS uses
/// the same helper from its public copy.
pub fn build_forgejo_workflow(_repo_root: &Utf8Path) -> Workflow {
    use common::*;

    let platform = CiPlatform::Forgejo;
    let targets = targets_for_platform(platform);
    let mut jobs = IndexMap::new();

    for target in &targets {
        let short = target.short_name();
        let is_linux = target.triple == "x86_64-unknown-linux-gnu";
        let job_id = if is_linux {
            "forgejo-linux-x64"
        } else {
            "forgejo-macos-arm64"
        };

        let archive_name = format!("dodeca-{}.{}", target.triple, target.archive_ext);
        let stable_key = format!("dodeca-{short}");
        let cargo_incremental = if is_linux { "1" } else { "0" };
        let maybe_check_ci = if is_linux {
            "cargo xtask ci-forgejo --check\n"
        } else {
            ""
        };
        let maybe_clean_incremental = if is_linux {
            ""
        } else {
            r#"rm -rf "$CARGO_TARGET_DIR"/release/incremental "$CARGO_TARGET_DIR"/debug/incremental
"#
        };
        let maybe_install_wasm_pack = if is_linux {
            ""
        } else {
            r#"if ! command -v wasm-pack >/dev/null 2>&1; then
  cargo install wasm-pack --locked
fi
"#
        };
        // The DevTools UI bundle serves /_/devtools/* with pnpm. vixen-ci ships
        // node but not pnpm, so without this build.rs silently ships an empty
        // DevTools/editor bundle and /_dodeca/edit/<page> is a blank page.
        // Linux is the release-critical platform (the kb installs the linux
        // ddc), so install loudly there; macOS is best-effort.
        let maybe_install_pnpm = if is_linux {
            "npm install -g pnpm\n"
        } else {
            "command -v pnpm >/dev/null 2>&1 || npm install -g pnpm || corepack enable || true\n"
        };
        // The DevTools UI package file-links `hotmeal-wasm`, whose pkg is
        // generated by wasm-pack. Build the absorbed Hotmeal wasm package before
        // build.rs runs the UI bundle. Linux only: that's the platform the kb
        // installs; macOS degrades to an empty UI (build still succeeds).
        let maybe_build_hotmeal = if is_linux {
            r#"HOTMEAL_DIR="$STABLE_SRC/libs/hotmeal"
(cd "$HOTMEAL_DIR/hotmeal-wasm" && wasm-pack build --target web --dev --target-dir target-wasm)
"#
        } else {
            ""
        };
        let maybe_browser_tests = if is_linux {
            r#"DODECA_BIN="$STABLE_SRC/target/release/ddc" \
  bash -lc 'cd "$STABLE_SRC/browser-tests" && npm ci && npx playwright install chromium --with-deps && npm test'
"#
        } else {
            ""
        };
        let maybe_xz_install = if target.triple.contains("apple") {
            r#"if ! command -v xz >/dev/null 2>&1 && command -v brew >/dev/null 2>&1; then
  HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1 brew install xz
fi
"#
        } else {
            ""
        };

        let install_helper = Step::run(
            "Install vixen-ci",
            r#"mkdir -p "$RUNNER_TEMP/vixen-ci"
curl -fsSL https://vixen-misc.s3-website.fr-par.scw.cloud/vixen-ci/latest/vixen-ci -o "$RUNNER_TEMP/vixen-ci/vixen-ci"
chmod +x "$RUNNER_TEMP/vixen-ci/vixen-ci"
echo "$RUNNER_TEMP/vixen-ci" >> "$GITHUB_PATH""#,
        );
        let install_rsync = if is_linux {
            Step::run(
                "Install Linux build tools",
                "apt-get update && apt-get install -y --no-install-recommends cmake rsync",
            )
        } else {
            Step::run("Check rsync", "rsync --version >/dev/null")
        };

        let prepare_stable_source = Step::run(
            "Prepare stable source",
            format!(
                r#"vixen-ci stable-src {stable_key} \
  --exclude=/crates/dodeca-devtools/pkg/ \
  --exclude=/crates/dodeca-search-wasm/pkg/
export STABLE_SRC="$HOME/.cache/vixen/{stable_key}-src"
export CARGO_TARGET_DIR="$HOME/.cache/vixen/{stable_key}-target"
cd "$STABLE_SRC"
rm -rf target
ln -s "$CARGO_TARGET_DIR" target
{maybe_clean_incremental}
echo "STABLE_SRC=$STABLE_SRC"
echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR""#
            ),
        );

        let build_and_test = Step::run(
            "Build and test",
            format!(
                r#"set -euo pipefail
cd "$STABLE_SRC"
{maybe_check_ci}
rustup target add wasm32-unknown-unknown
{maybe_install_wasm_pack}{maybe_install_pnpm}{maybe_build_hotmeal}# Force a clean wasm rebuild before compiling ddc (which embeds the search +
# devtools wasm via include_bytes!). build.rs skips when pkg/ exists, and the
# stable-src cache preserves a stale pkg/ across runs — that combination
# shipped an out-of-date search reader in v0.14.4. Removing it makes build.rs
# regenerate from the current source on every release.
rm -rf crates/dodeca-devtools/pkg crates/dodeca-search-wasm/pkg
cargo nextest run
cargo xtask integration
if [[ "${{GITHUB_REF_TYPE:-}}" == "tag" && -n "${{GITHUB_REF_NAME:-}}" ]]; then
  export DODECA_RELEASE_VERSION="${{GITHUB_REF_NAME}}"
fi
cargo build --release
actual="$(target/release/ddc --version)"
echo "$actual"
if [[ "${{GITHUB_REF_TYPE:-}}" == "tag" && -n "${{GITHUB_REF_NAME:-}}" ]]; then
  expected="ddc ${{GITHUB_REF_NAME#v}}"
  if [[ "$actual" != "$expected" ]]; then
    echo "Expected '$expected', got '$actual'" >&2
    exit 1
  fi
fi
rm -rf dist
mkdir -p dist
cp target/release/ddc dist/
chmod +x dist/ddc
{verify}
{maybe_browser_tests}"#,
                verify = verify_artifacts_script(),
            ),
        )
        .shell("bash");

        let assemble = Step::run(
            "Assemble archive",
            format!(
                r#"set -euo pipefail
cd "$STABLE_SRC"
{maybe_xz_install}bash scripts/assemble-archive.sh {triple}
mkdir -p "$GITHUB_WORKSPACE/dist"
cp "{archive_name}" "$GITHUB_WORKSPACE/dist/"
ls -la "$GITHUB_WORKSPACE/dist/""#,
                triple = target.triple,
            ),
        )
        .shell("bash");

        let mut steps = Vec::new();
        if !is_linux {
            steps.push(install_helper);
        }
        steps.push(
            Step::run("Prepare Vixen CI", "vixen-ci prepare")
                .with_env([("DEPLOY_KEY", "${{ secrets.DEPLOY_KEY }}")]),
        );
        steps.push(Step::run("Checkout", "vixen-ci checkout"));
        if !is_linux {
            steps.push(Step::run("Rust toolchain", "vixen-ci rust-toolchain"));
            steps.push(Step::run("Install nextest", "vixen-ci nextest"));
            steps.push(install_rsync);
        }
        steps.push(prepare_stable_source);
        steps.push(build_and_test);
        steps.push(assemble);
        steps.push(upload_artifact(
            platform,
            format!("build-{short}"),
            format!("dist/{archive_name}"),
        ));

        let mut job = Job::with_runner(target.runs_on())
            .name(format!("Forgejo {short}"))
            .timeout(90)
            .env([
                ("CARGO_INCREMENTAL", cargo_incremental),
                ("FORGEJO_TOKEN", "${{ github.token }}"),
                ("RUST_BACKTRACE", "1"),
            ])
            .steps(steps);

        if is_linux {
            job = job
                .container(
                    "code.vixen.rs/vixen/vixen-ci:latest",
                    ["/srv/ci/cache/dodeca:/srv/ci/cache/dodeca"],
                )
                .env([
                    ("HOME", "/srv/ci/cache/dodeca/home"),
                    ("CARGO_HOME", "/srv/ci/cache/dodeca/cargo"),
                    ("CARGO_INCREMENTAL", cargo_incremental),
                    ("FORGEJO_TOKEN", "${{ github.token }}"),
                    ("RUST_BACKTRACE", "1"),
                ]);
        }

        jobs.insert(job_id.to_string(), job);
    }

    // Release publish job. On a tag build only, after both platforms are built,
    // download the two archives and push them to the Scaleway bucket the
    // installer reads from (scripts/publish-release.sh). This mirrors how
    // vixen-ci publishes itself: upload via the S3 API endpoint with
    // public-read, served via the website endpoint (RELEASE_BASE_URL).
    // Credentials are the ACCESS_KEY_ID / ACCESS_SECRET_KEY Forgejo secrets
    // (the Scaleway Object Storage key, same as vixen-ci's publish step).
    let publish = Job::with_runner(linux_runner(platform).to_runs_on())
        .name("Publish release")
        .timeout(30)
        .needs(["forgejo-linux-x64", "forgejo-macos-arm64"])
        .if_condition("startsWith(github.ref, 'refs/tags/')")
        .container(
            "code.vixen.rs/vixen/vixen-ci:latest",
            ["/srv/ci/cache/dodeca:/srv/ci/cache/dodeca"],
        )
        .env([
            ("HOME", "/srv/ci/cache/dodeca/home"),
            ("FORGEJO_TOKEN", "${{ github.token }}"),
        ])
        .steps([
            Step::run(
                "Checkout repository",
                // Org-agnostic: derive the clone URL from the runner context so
                // it keeps working regardless of which Forgejo org owns the
                // repo. Only needs install.sh + scripts/, so a shallow fetch of
                // the tagged commit is enough.
                r#"mkdir -p "$GITHUB_WORKSPACE"
cd "$GITHUB_WORKSPACE"
git init -q
git remote remove origin 2>/dev/null || true
git remote add origin "${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}.git"
git -c http.extraHeader="Authorization: token ${FORGEJO_TOKEN}" fetch --depth 1 origin "${GITHUB_SHA}"
git checkout -q --detach FETCH_HEAD"#,
            )
            .shell("bash"),
            download_artifact(platform, "build-linux-x64", "dist"),
            download_artifact(platform, "build-macos-arm64", "dist"),
            // Forgejo's download-artifact may nest each artifact in a subdir;
            // hoist every file to dist/ so scripts/release.sh finds the tarballs.
            Step::run(
                "Flatten artifact directories",
                "find dist -mindepth 2 -type f -exec mv -t dist {} + 2>/dev/null || true; find dist -mindepth 1 -type d -empty -delete 2>/dev/null || true",
            ),
            Step::run("List artifacts", "ls -la dist/"),
            Step::run("Prepare release artifacts", "bash scripts/release.sh").shell("bash"),
            Step::run(
                "Publish to object storage",
                r#"bash scripts/publish-release.sh "${GITHUB_REF_NAME}""#,
            )
            .shell("bash")
            .with_env([
                ("AWS_ACCESS_KEY_ID", "${{ secrets.ACCESS_KEY_ID }}"),
                ("AWS_SECRET_ACCESS_KEY", "${{ secrets.ACCESS_SECRET_KEY }}"),
                ("AWS_DEFAULT_REGION", "fr-par"),
                ("AWS_EC2_METADATA_DISABLED", "true"),
            ]),
        ]);
    jobs.insert("publish".to_string(), publish);

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
            // Forgejo doesn't support merge_group yet
            merge_group: None,
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
use eyre::Result;

const GENERATED_HEADER: &str =
    "# GENERATED BY: cargo xtask ci\n# DO NOT EDIT - edit xtask/src/ci.rs instead\n\n";

// =============================================================================
// Installer scripts
// =============================================================================

/// Base URL for release artifacts: the `bearcove-dist` Scaleway Object Storage
/// bucket (fr-par), under the `dodeca/releases/` prefix. dodeca is a bearcove
/// project, so it has its own bucket separate from Vixen's. Each release is
/// `<base>/<version>/dodeca-<platform>.tar.xz`; `<base>/latest` is a text file
/// holding the newest version string. Overridable at install time via
/// `DODECA_BASE_URL` (mirrors / testing).
///
/// Objects are uploaded public-read and served from the same S3 API endpoint
/// (`s3.fr-par.scw.cloud`); see scripts/publish-release.sh.
pub const RELEASE_BASE_URL: &str = "https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/releases";

/// Generate the shell installer script content.
pub fn generate_installer_script() -> String {
    // No Rust interpolation needed — everything below is shell. The default
    // base URL is injected once so the generator stays the single source.
    format!(
        r##"#!/bin/sh
# Installer for dodeca
# Usage: curl -fsSL https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/install.sh | sh

set -eu

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override BASE_URL for a mirror or local testing; DODECA_VERSION pins a
# specific version (otherwise the `latest` pointer is read).
BASE_URL="${{DODECA_BASE_URL:-{base_url}}}"

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

# Read the `latest` pointer (a text file holding the newest version string).
get_latest_version() {{
    curl -fsSL "$BASE_URL/latest"
}}

main() {{
    local platform version archive_name url install_dir

    platform="$(detect_platform)"
    version="${{DODECA_VERSION:-$(get_latest_version)}}"
    archive_name="dodeca-$platform.tar.xz"
    url="$BASE_URL/$version/$archive_name"
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
        base_url = RELEASE_BASE_URL
    )
}

/// Generate the PowerShell installer script content.
pub fn generate_powershell_installer() -> String {
    format!(
        r##"# Installer for dodeca
# Usage: powershell -ExecutionPolicy Bypass -c "irm https://bearcove-dist.s3.fr-par.scw.cloud/dodeca/install.ps1 | iex"

$ErrorActionPreference = 'Stop'

# Release artifacts live in a Scaleway Object Storage bucket we control.
# Override with $env:DODECA_BASE_URL; $env:DODECA_VERSION pins a version.
$BaseUrl = if ($env:DODECA_BASE_URL) {{ $env:DODECA_BASE_URL }} else {{ "{base_url}" }}

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
        return (Invoke-RestMethod -Uri "$BaseUrl/latest").Trim()
    }} catch {{
        Write-Error "Failed to get latest version: $_"
        exit 1
    }}
}}

function Main {{
    $arch = Get-Architecture
    $version = if ($env:DODECA_VERSION) {{ $env:DODECA_VERSION }} else {{ Get-LatestVersion }}
    $archiveName = "dodeca-x86_64-pc-windows-msvc.zip"
    $url = "$BaseUrl/$version/$archiveName"

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
        base_url = RELEASE_BASE_URL
    )
}

/// Helper to serialize a workflow to YAML with the generated header.
fn workflow_to_yaml(workflow: &Workflow) -> Result<String> {
    let yaml = facet_yaml::to_string(workflow)
        .map_err(|e| eyre::eyre!("failed to serialize workflow: {}", e))?;
    let yaml = yaml
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!("{GENERATED_HEADER}{yaml}\n"))
}

/// Check or write a generated file.
fn check_or_write(path: &Utf8Path, content: &str, check: bool) -> Result<()> {
    if check {
        let existing = fs_err::read_to_string(path)
            .map_err(|e| eyre::eyre!("failed to read {}: {}", path, e))?;

        if existing != content {
            return Err(eyre::eyre!(
                "{} is out of date. Run `cargo xtask ci-github` and `cargo xtask ci-forgejo` to update.",
                path.file_name().unwrap_or("file")
            ));
        }
        println!("{} is up to date.", path.file_name().unwrap_or("file"));
    } else {
        fs_err::create_dir_all(path.parent().unwrap())
            .map_err(|e| eyre::eyre!("failed to create directory: {}", e))?;

        fs_err::write(path, content).map_err(|e| eyre::eyre!("failed to write {}: {}", path, e))?;

        println!("Generated: {}", path);
    }
    Ok(())
}

/// Generate GitHub Actions workflow and installer script.
pub fn generate_github(repo_root: &Utf8Path, check: bool) -> Result<()> {
    // Generate GitHub Actions workflow
    let github_workflows_dir = repo_root.join(CiPlatform::GitHub.workflows_dir());
    let github_ci_workflow = build_ci_workflow(CiPlatform::GitHub, repo_root);
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
    let forgejo_ci_workflow = build_forgejo_workflow(repo_root);
    let forgejo_ci_yaml = workflow_to_yaml(&forgejo_ci_workflow)?;
    check_or_write(
        &forgejo_workflows_dir.join("ci.yml"),
        &forgejo_ci_yaml,
        check,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;

    #[test]
    fn github_release_needs_linux_integration_and_tag_only_packages() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));

        let release = workflow.jobs.get("release").expect("release job exists");
        assert_eq!(
            release.needs.as_ref(),
            Some(&vec![
                "integration-linux-x64".to_string(),
                "package-linux-x64".to_string(),
                "package-macos-arm64".to_string(),
                "package-windows-x64".to_string(),
            ])
        );
    }

    #[test]
    fn github_package_jobs_are_tag_only_on_intended_runners() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));

        let linux_pkg = workflow
            .jobs
            .get("package-linux-x64")
            .expect("linux package job");
        assert_eq!(
            linux_pkg.runs_on.0,
            vec!["bearcove-ubuntu-24.04".to_string()]
        );
        assert_eq!(
            linux_pkg.if_condition.as_deref(),
            Some("startsWith(github.ref, 'refs/tags/')")
        );

        let mac_pkg = workflow
            .jobs
            .get("package-macos-arm64")
            .expect("mac package job");
        assert_eq!(
            mac_pkg.runs_on.0,
            vec!["blacksmith-12vcpu-macos-15".to_string()]
        );
        assert_eq!(
            mac_pkg.if_condition.as_deref(),
            Some("startsWith(github.ref, 'refs/tags/')")
        );

        let win_pkg = workflow
            .jobs
            .get("package-windows-x64")
            .expect("windows package job");
        assert_eq!(
            win_pkg.runs_on.0,
            vec!["blacksmith-8vcpu-windows-2025".to_string()]
        );
        assert_eq!(
            win_pkg.if_condition.as_deref(),
            Some("startsWith(github.ref, 'refs/tags/')")
        );
    }

    #[test]
    fn github_every_commit_checks_on_free_runners() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));

        let mac = workflow.jobs.get("check-macos").expect("mac check job");
        assert_eq!(mac.runs_on.0, vec!["macos-15".to_string()]);
        assert!(mac.if_condition.is_none());
        assert_eq!(
            mac.steps
                .iter()
                .find(|s| s.name.as_deref() == Some("Check macOS"))
                .and_then(|s| s.run.as_deref()),
            Some("cargo check --workspace --all-targets")
        );

        let win = workflow
            .jobs
            .get("check-windows")
            .expect("windows check job");
        assert_eq!(win.runs_on.0, vec!["windows-latest".to_string()]);
        assert!(win.if_condition.is_none());
        let win_rust_toolchain = win
            .steps
            .iter()
            .find(|s| s.name.as_deref() == Some("Install Rust"))
            .and_then(|s| s.with.as_ref().and_then(|i| i.get("toolchain")));
        assert_eq!(win_rust_toolchain, Some(&"stable".to_string()));
        assert_eq!(
            win.steps
                .iter()
                .find(|s| s.name.as_deref() == Some("Check Windows"))
                .and_then(|s| s.run.as_deref()),
            Some("cargo check --workspace --all-targets")
        );
    }

    #[test]
    fn github_no_macos_build_or_integration_on_commit() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));
        assert!(!workflow.jobs.contains_key("build-ddc-macos-arm64"));
        assert!(!workflow.jobs.contains_key("integration-macos-arm64"));
        assert!(!workflow.jobs.contains_key("release-macos-arm64"));
    }

    #[test]
    fn github_linux_build_does_not_archive_on_commit() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));
        let build = workflow
            .jobs
            .get("build-ddc-linux-x64")
            .expect("linux build job");
        // No "Assemble archive" step on the every-commit build.
        assert!(
            !build
                .steps
                .iter()
                .any(|s| s.name.as_deref() == Some("Assemble archive"))
        );
    }

    #[test]
    fn github_all_rust_install_steps_pin_stable() {
        let workflow = build_ci_workflow(CiPlatform::GitHub, Utf8Path::new("."));
        for (id, job) in &workflow.jobs {
            for step in &job.steps {
                if step.name.as_deref() == Some("Install Rust") {
                    let toolchain = step
                        .with
                        .as_ref()
                        .and_then(|i| i.get("toolchain"))
                        .unwrap_or_else(|| {
                            panic!("job {id} Install Rust step missing toolchain input")
                        });
                    assert_eq!(toolchain, "stable", "job {id} should pin stable toolchain");
                }
            }
        }
    }
}
