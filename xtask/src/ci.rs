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

/// Generate CI workflow files and installer script.
pub fn generate(repo_root: &Utf8Path, check: bool) -> Result<()> {
    // Generate release workflow
    let workflow = build_release_workflow();
    let yaml_content = format!(
        "{}{}",
        GENERATED_HEADER,
        facet_yaml::to_string(&workflow)
            .map_err(|e| miette::miette!("failed to serialize workflow: {}", e))?
    );

    let release_path = repo_root.join(".github/workflows/release.yml");

    // Generate installer script
    let installer_content = generate_installer_script();
    let installer_path = repo_root.join("install.sh");

    if check {
        // Check release workflow
        let existing = fs_err::read_to_string(&release_path)
            .map_err(|e| miette::miette!("failed to read {}: {}", release_path, e))?;

        if existing != yaml_content {
            return Err(miette::miette!(
                "Release workflow is out of date. Run `cargo xtask ci` to update."
            ));
        }
        println!("Release workflow is up to date.");

        // Check installer script
        let existing_installer = fs_err::read_to_string(&installer_path)
            .map_err(|e| miette::miette!("failed to read {}: {}", installer_path, e))?;

        if existing_installer != installer_content {
            return Err(miette::miette!(
                "Installer script is out of date. Run `cargo xtask ci` to update."
            ));
        }
        println!("Installer script is up to date.");
    } else {
        // Write release workflow
        fs_err::create_dir_all(release_path.parent().unwrap())
            .map_err(|e| miette::miette!("failed to create directory: {}", e))?;

        fs_err::write(&release_path, &yaml_content)
            .map_err(|e| miette::miette!("failed to write {}: {}", release_path, e))?;

        println!("Generated: {}", release_path);

        // Write installer script
        fs_err::write(&installer_path, &installer_content)
            .map_err(|e| miette::miette!("failed to write {}: {}", installer_path, e))?;

        println!("Generated: {}", installer_path);
    }

    Ok(())
}
