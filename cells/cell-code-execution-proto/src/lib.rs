//! RPC protocol for dodeca code execution cell
//!
//! Defines services for extracting and executing code samples from markdown.

use facet::Facet;

// ============================================================================
// Configuration Types
// ============================================================================

/// Code execution configuration.
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct CodeExecutionConfig {
    /// Enable/disable code execution
    #[facet(default)]
    pub enabled: Option<bool>,

    /// Fail build on execution errors
    #[facet(default)]
    pub fail_on_error: Option<bool>,

    /// Execution timeout in seconds
    #[facet(default)]
    pub timeout_secs: Option<u64>,

    /// Cache directory for execution artifacts
    #[facet(default)]
    pub cache_dir: Option<String>,

    /// Project root directory (for resolving path dependencies)
    /// Set at runtime, not from config
    #[facet(default)]
    pub project_root: Option<String>,

    /// Dependencies for code samples
    #[facet(default)]
    pub dependencies: Option<Vec<DependencySpec>>,

    /// Language-specific configuration
    #[facet(default)]
    pub rust: Option<RustConfig>,
}

impl CodeExecutionConfig {
    /// Whether code execution is enabled (default: true)
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Whether to fail build on execution errors (default: true)
    pub fn should_fail_on_error(&self) -> bool {
        self.fail_on_error.unwrap_or(true)
    }

    /// Execution timeout in seconds (default: 30)
    pub fn timeout(&self) -> u64 {
        self.timeout_secs.unwrap_or(30)
    }

    /// Cache directory (default: ".cache/code-execution")
    pub fn cache_directory(&self) -> String {
        self.cache_dir
            .clone()
            .unwrap_or_else(|| ".cache/code-execution".to_string())
    }

    /// Project root directory
    pub fn project_root(&self) -> Option<&str> {
        self.project_root.as_deref()
    }

    /// Get dependencies (default: empty)
    pub fn dependencies(&self) -> &[DependencySpec] {
        self.dependencies.as_deref().unwrap_or(&[])
    }
}

/// A single dependency specification
///
/// Supports crates.io, git, and path dependencies.
///
/// Example YAML:
/// ```yaml
/// dependencies:
///   - name: serde
///     version: "1.0"
///   - name: serde
///     version: "1.0"
///     features:
///       - derive
///   - name: facet
///     version: "0.1"
///     git: https://github.com/facet-rs/facet
///     branch: main
///   - name: plugcard
///     version: "0.1"
///     path: crates/plugcard
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DependencySpec {
    /// Crate name
    pub name: String,

    /// Version requirement
    pub version: String,

    /// Git repository URL (optional)
    #[facet(default)]
    pub git: Option<String>,

    /// Git revision/commit hash (optional)
    #[facet(default)]
    pub rev: Option<String>,

    /// Git branch (optional)
    #[facet(default)]
    pub branch: Option<String>,

    /// Local path (optional, relative to project root)
    #[facet(default)]
    pub path: Option<String>,

    /// Crate features to enable (optional)
    #[facet(default)]
    pub features: Option<Vec<String>>,
}

impl DependencySpec {
    /// Create a new crates.io dependency
    pub fn crates_io(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            git: None,
            rev: None,
            branch: None,
            path: None,
            features: None,
        }
    }

    /// Create a new git dependency with branch
    pub fn git_branch(
        name: impl Into<String>,
        git: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: "0.0.0".into(), // Version is ignored for git deps
            git: Some(git.into()),
            rev: None,
            branch: Some(branch.into()),
            path: None,
            features: None,
        }
    }

    /// Create a new path dependency
    pub fn path(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: "0.0.0".into(), // Version is ignored for path deps
            git: None,
            rev: None,
            branch: None,
            path: Some(path.into()),
            features: None,
        }
    }

    /// Add features to this dependency
    pub fn with_features(mut self, features: Vec<String>) -> Self {
        self.features = Some(features);
        self
    }

    /// Generate the Cargo.toml dependency line for this spec
    pub fn to_cargo_toml_line(&self) -> String {
        self.to_cargo_toml_line_with_root(None)
    }

    /// Generate the Cargo.toml dependency line with optional project root for path resolution
    pub fn to_cargo_toml_line_with_root(&self, project_root: Option<&std::path::Path>) -> String {
        let mut parts = Vec::new();

        if let Some(ref path) = self.path {
            // Path dependency - make absolute if project_root provided
            let resolved_path = if let Some(root) = project_root {
                root.join(path).display().to_string()
            } else {
                path.clone()
            };
            parts.push(format!("path = \"{}\"", resolved_path));
        } else if let Some(ref git) = self.git {
            parts.push(format!("git = \"{}\"", git));
            if let Some(ref rev) = self.rev {
                parts.push(format!("rev = \"{}\"", rev));
            } else if let Some(ref branch) = self.branch {
                parts.push(format!("branch = \"{}\"", branch));
            }
        } else {
            parts.push(format!("version = \"{}\"", self.version));
        }

        if let Some(ref features) = self.features
            && !features.is_empty()
        {
            let features_str = features
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("features = [{}]", features_str));
        }

        format!("{} = {{ {} }}", self.name, parts.join(", "))
    }
}

/// Rust-specific configuration
///
/// Example YAML:
/// ```yaml
/// rust:
///   command: cargo
///   args:
///     - run
///     - --quiet
///     - --release
///   extension: rs
///   prepare_code: true
///   auto_imports:
///     - "use std::*;"
///   show_output: true
/// ```
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct RustConfig {
    /// Cargo command
    #[facet(default)]
    pub command: Option<String>,

    /// Cargo arguments
    #[facet(default)]
    pub args: Option<Vec<String>>,

    /// File extension
    #[facet(default)]
    pub extension: Option<String>,

    /// Auto-wrap code without main function
    #[facet(default)]
    pub prepare_code: Option<bool>,

    /// Auto-imports
    #[facet(default)]
    pub auto_imports: Option<Vec<String>>,

    /// Show output in build
    #[facet(default)]
    pub show_output: Option<bool>,
}

/// Per-language execution configuration (runtime)
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageConfig {
    /// Command to run for this language
    pub command: String,
    /// Arguments to pass to the command
    pub args: Vec<String>,
    /// File extension for temporary files
    pub extension: String,
    /// Prepare code before execution (e.g., add main function)
    pub prepare_code: bool,
    /// Auto-imports to add to every code sample
    pub auto_imports: Vec<String>,
    /// Show output even on success
    pub show_output: bool,
    /// Expected compilation errors (regex patterns)
    pub expected_compile_errors: Vec<String>,
}

impl LanguageConfig {
    /// Create default Rust language config
    pub fn rust() -> Self {
        Self {
            command: "cargo".to_string(),
            args: vec!["run".to_string(), "--release".to_string()],
            extension: "rs".to_string(),
            prepare_code: true,
            auto_imports: vec![
                "use std::collections::HashMap;".to_string(),
                "use facet::Facet;".to_string(),
            ],
            show_output: true,
            expected_compile_errors: vec![],
        }
    }

    /// Create from RustConfig (YAML parsed)
    pub fn from_rust_config(rust: &RustConfig) -> Self {
        Self {
            command: rust.command.clone().unwrap_or_else(|| "cargo".to_string()),
            args: rust
                .args
                .clone()
                .unwrap_or_else(|| vec!["run".to_string(), "--release".to_string()]),
            extension: rust.extension.clone().unwrap_or_else(|| "rs".to_string()),
            prepare_code: rust.prepare_code.unwrap_or(true),
            auto_imports: rust.auto_imports.clone().unwrap_or_else(|| {
                vec![
                    "use std::collections::HashMap;".to_string(),
                    "use facet::Facet;".to_string(),
                ]
            }),
            show_output: rust.show_output.unwrap_or(true),
            expected_compile_errors: vec![],
        }
    }
}

/// Default dependencies for Rust code samples
pub fn default_rust_dependencies() -> Vec<DependencySpec> {
    vec![
        DependencySpec::git_branch("facet", "https://github.com/facet-rs/facet", "main"),
        DependencySpec::git_branch("facet-json", "https://github.com/facet-rs/facet", "main"),
    ]
}

// ============================================================================
// Code Sample Types
// ============================================================================

/// A code sample extracted from markdown
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodeSample {
    /// The source file this came from
    pub source_path: String,
    /// Line number in the source file
    pub line: usize,
    /// Programming language
    pub language: String,
    /// The raw code content
    pub code: String,
    /// Whether this sample should be executed
    pub executable: bool,
    /// Expected compilation errors (from code block metadata)
    pub expected_errors: Vec<String>,
}

/// Status of code sample execution
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExecutionStatus {
    /// Code was executed and succeeded
    Success,
    /// Code was executed and failed
    Failed,
    /// Code was not executed (noexec, non-Rust, etc.)
    Skipped,
}

/// Result of executing a code sample
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecutionResult {
    /// Execution status (Success, Failed, or Skipped)
    pub status: ExecutionStatus,
    /// Exit code (if executed)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution duration
    pub duration_ms: u64,
    /// Error message if execution failed
    pub error: Option<String>,
    /// Build metadata for reproducibility
    pub metadata: Option<BuildMetadata>,
}

/// Build metadata captured for reproducibility
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildMetadata {
    /// Rust compiler version (from `rustc --version --verbose`)
    pub rustc_version: String,
    /// Cargo version (from `cargo --version`)
    pub cargo_version: String,
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target: String,
    /// Build timestamp (ISO 8601 format)
    pub timestamp: String,
    /// Whether shared target cache was used (vs fresh build)
    pub cache_hit: bool,
    /// Platform (e.g., "linux", "macos", "windows")
    pub platform: String,
    /// CPU architecture (e.g., "x86_64", "aarch64")
    pub arch: String,
    /// Dependencies with exact resolved versions (from Cargo.lock)
    pub dependencies: Vec<ResolvedDependency>,
}

/// A resolved dependency with exact version info
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedDependency {
    /// Crate name
    pub name: String,
    /// Exact version
    pub version: String,
    /// Source of the dependency
    pub source: DependencySource,
}

/// Source of a resolved dependency
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DependencySource {
    /// crates.io registry
    CratesIo,
    /// Git repository with commit hash
    Git { url: String, commit: String },
    /// Local path
    Path { path: String },
}

// ============================================================================
// RPC Input/Output Types
// ============================================================================

/// Input for extracting code samples
#[derive(Facet)]
pub struct ExtractSamplesInput {
    /// Source file path
    pub source_path: String,
    /// Markdown content
    pub content: String,
}

/// Output from extracting code samples
#[derive(Facet, Debug, Clone)]
pub struct ExtractSamplesOutput {
    /// Extracted code samples
    pub samples: Vec<CodeSample>,
}

/// Input for executing code samples
#[derive(Facet)]
pub struct ExecuteSamplesInput {
    /// Code samples to execute
    pub samples: Vec<CodeSample>,
    /// Execution configuration
    pub config: CodeExecutionConfig,
}

/// Output from executing code samples
#[derive(Facet, Debug, Clone)]
pub struct ExecuteSamplesOutput {
    /// Execution results
    pub results: Vec<(CodeSample, ExecutionResult)>,
}

// ============================================================================
// RPC Service
// ============================================================================

/// Result of code execution operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum CodeExecutionResult {
    /// Successfully extracted samples
    ExtractSuccess { output: ExtractSamplesOutput },
    /// Successfully executed samples
    ExecuteSuccess { output: ExecuteSamplesOutput },
    /// Error during processing
    Error { message: String },
}

/// Code execution service implemented by the cell.
///
/// The host calls these methods to process code samples.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait CodeExecutor {
    /// Extract code samples from markdown content
    async fn extract_code_samples(&self, input: ExtractSamplesInput) -> CodeExecutionResult;

    /// Execute code samples
    async fn execute_code_samples(&self, input: ExecuteSamplesInput) -> CodeExecutionResult;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crates_io_dep() {
        let dep = DependencySpec::crates_io("serde", "1.0");
        assert_eq!(dep.to_cargo_toml_line(), "serde = { version = \"1.0\" }");
    }

    #[test]
    fn test_crates_io_with_features() {
        let dep =
            DependencySpec::crates_io("serde", "1.0").with_features(vec!["derive".to_string()]);
        assert_eq!(
            dep.to_cargo_toml_line(),
            "serde = { version = \"1.0\", features = [\"derive\"] }"
        );
    }

    #[test]
    fn test_git_branch_dep() {
        let dep = DependencySpec::git_branch("facet", "https://github.com/facet-rs/facet", "main");
        assert_eq!(
            dep.to_cargo_toml_line(),
            "facet = { git = \"https://github.com/facet-rs/facet\", branch = \"main\" }"
        );
    }

    #[test]
    fn test_path_dep() {
        let dep = DependencySpec::path("plugcard", "crates/plugcard");
        assert_eq!(
            dep.to_cargo_toml_line(),
            "plugcard = { path = \"crates/plugcard\" }"
        );
    }

    #[test]
    fn test_path_dep_with_root() {
        let dep = DependencySpec::path("plugcard", "crates/plugcard");
        let root = std::path::Path::new("/home/user/project");
        assert_eq!(
            dep.to_cargo_toml_line_with_root(Some(root)),
            "plugcard = { path = \"/home/user/project/crates/plugcard\" }"
        );
    }

    #[test]
    fn test_default_dependencies() {
        let deps = default_rust_dependencies();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "facet");
        assert_eq!(deps[1].name, "facet-json");
    }

    #[test]
    fn test_rust_config_yaml() {
        let yaml = r#"
command: cargo
args:
  - run
  - --quiet
  - --release
extension: rs
prepare_code: true
auto_imports:
  - "use std::collections::HashMap;"
show_output: true
"#;

        let config: RustConfig = facet_yaml::from_str(yaml).unwrap();

        assert_eq!(config.command, Some("cargo".to_string()));
        assert_eq!(config.extension, Some("rs".to_string()));
        assert_eq!(config.prepare_code, Some(true));
        assert_eq!(config.show_output, Some(true));
        assert_eq!(
            config.args,
            Some(vec![
                "run".to_string(),
                "--quiet".to_string(),
                "--release".to_string()
            ])
        );
        assert_eq!(
            config.auto_imports,
            Some(vec!["use std::collections::HashMap;".to_string()])
        );
    }
}
