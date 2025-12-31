//! Shared types for dodeca code execution plugin interface
//!
//! This crate contains the type definitions used by both the main dodeca binary
//! and the code-execution plugin. By separating types from implementation,
//! the main binary doesn't need to link against the heavy plugin code.

use facet::Facet;
use std::collections::HashMap;

// Re-export config types from the config crate
pub use dodeca_code_execution_config::{
    CodeExecutionConfig as KdlCodeExecutionConfig, DependenciesConfig, DependencySpec, MultiValue,
    RustConfig, SingleValue, default_rust_dependencies,
};

/// Runtime configuration for code sample execution (used by the plugin)
#[derive(Facet, Debug, Clone, PartialEq, Eq)]
pub struct CodeExecutionConfig {
    /// Enable code sample execution
    pub enabled: bool,
    /// Fail build on execution errors (vs just warnings in dev)
    pub fail_on_error: bool,
    /// Timeout for code execution (seconds)
    pub timeout_secs: u64,
    /// Cache directory for execution results (relative to project root)
    pub cache_dir: String,
    /// Project root directory (for resolving path dependencies)
    pub project_root: Option<String>,
    /// Languages to execute (empty = all supported)
    pub languages: Vec<String>,
    /// Dependencies available to all code samples
    pub dependencies: Vec<DependencySpec>,
    /// Per-language configuration
    pub language_config: HashMap<String, LanguageConfig>,
}

impl Default for CodeExecutionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fail_on_error: true,
            timeout_secs: 30,
            cache_dir: ".cache/code-execution".to_string(),
            project_root: None,
            languages: vec!["rust".to_string()],
            dependencies: default_rust_dependencies(),
            language_config: HashMap::from([("rust".to_string(), LanguageConfig::rust())]),
        }
    }
}

impl CodeExecutionConfig {
    /// Create from KDL config, applying defaults for unspecified values
    pub fn from_kdl_config(kdl: &KdlCodeExecutionConfig) -> Self {
        Self::from_kdl_config_with_root(kdl, None)
    }

    /// Create from KDL config with a project root for resolving path dependencies
    pub fn from_kdl_config_with_root(
        kdl: &KdlCodeExecutionConfig,
        project_root: Option<String>,
    ) -> Self {
        let defaults = Self::default();

        // Use user-specified deps if any, otherwise use defaults
        let dependencies = if kdl.dependencies.deps.is_empty() {
            defaults.dependencies
        } else {
            kdl.dependencies.deps.clone()
        };

        // Build language config from rust settings
        let mut language_config = HashMap::new();
        let rust_config = LanguageConfig {
            command: kdl
                .rust
                .command
                .as_ref()
                .map(|c| c.value.clone())
                .unwrap_or_else(|| "cargo".to_string()),
            args: kdl
                .rust
                .args
                .as_ref()
                .map(|a| a.values.clone())
                .unwrap_or_else(|| vec!["run".to_string(), "--release".to_string()]),
            extension: kdl
                .rust
                .extension
                .as_ref()
                .map(|e| e.value.clone())
                .unwrap_or_else(|| "rs".to_string()),
            prepare_code: kdl
                .rust
                .prepare_code
                .as_ref()
                .map(|p| p.value)
                .unwrap_or(true),
            auto_imports: kdl
                .rust
                .auto_imports
                .as_ref()
                .map(|a| a.values.clone())
                .unwrap_or_else(|| {
                    vec![
                        "use std::collections::HashMap;".to_string(),
                        "use facet::Facet;".to_string(),
                    ]
                }),
            show_output: kdl
                .rust
                .show_output
                .as_ref()
                .map(|s| s.value)
                .unwrap_or(true),
            expected_compile_errors: vec![],
        };
        language_config.insert("rust".to_string(), rust_config);

        Self {
            enabled: kdl.enabled.unwrap_or(true),
            fail_on_error: kdl.fail_on_error.unwrap_or(true),
            timeout_secs: kdl.timeout_secs.unwrap_or(30),
            cache_dir: kdl
                .cache_dir
                .clone()
                .unwrap_or_else(|| ".cache/code-execution".to_string()),
            project_root,
            languages: vec!["rust".to_string()],
            dependencies,
            language_config,
        }
    }
}

/// Per-language execution configuration
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
}

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
