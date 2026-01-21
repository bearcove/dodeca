//! Configuration types for dodeca static site generator.
//!
//! This crate contains the configuration structs that are parsed from
//! `.config/dodeca.styx` (or `.config/dodeca.yaml`).

use std::collections::HashMap;

use facet::Facet;

// Re-export code execution config
pub use cell_code_execution_proto::CodeExecutionConfig;
// Re-export Schema for build step param types
pub use facet_styx::Schema;

/// Dodeca configuration from `.config/dodeca.styx`
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "snake_case")]
pub struct DodecaConfig {
    /// Base URL for the site (e.g., `https://example.com`)
    /// Used to generate permalinks. Defaults to "/" for local development.
    #[facet(default)]
    pub base_url: Option<String>,

    /// Content directory (relative to project root)
    pub content: String,

    /// Output directory (relative to project root)
    pub output: String,

    /// Link checking configuration
    #[facet(default)]
    pub link_check: Option<LinkCheckConfig>,

    /// Assets that should be served at their original paths (no cache-busting)
    /// e.g., favicon.svg, robots.txt, og-image.png
    #[facet(default)]
    pub stable_assets: Option<Vec<String>>,

    /// Code execution configuration
    #[facet(default)]
    pub code_execution: Option<CodeExecutionConfig>,

    /// Syntax highlighting theme configuration
    #[facet(default)]
    pub syntax_highlight: Option<SyntaxHighlightConfig>,

    /// Build steps - parameterized commands invoked from templates.
    /// Keys are step names, values define params and command.
    #[facet(default)]
    pub build_steps: Option<HashMap<String, BuildStepDef>>,
}

/// Syntax highlighting theme configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct SyntaxHighlightConfig {
    /// Light theme name (e.g., "github-light", "catppuccin-latte")
    #[facet(default)]
    pub light_theme: Option<String>,

    /// Dark theme name (e.g., "tokyo-night", "catppuccin-mocha")
    #[facet(default)]
    pub dark_theme: Option<String>,
}

/// Link checking configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct LinkCheckConfig {
    /// Domains to skip checking (anti-bot policies, known flaky, etc.)
    #[facet(default)]
    pub skip_domains: Option<Vec<String>>,

    /// Minimum delay between requests to the same domain (milliseconds)
    /// Default: 1000ms (1 second)
    #[facet(default)]
    pub rate_limit_ms: Option<u64>,
}

/// A build step definition.
///
/// Build steps are parameterized commands that can be invoked from templates.
/// Parameters can be typed (e.g., `@file`, `@int`, `@string`) and `@file` params
/// are tracked for caching - the step re-runs when file contents change.
///
/// Example in `.config/dodeca.styx`:
/// ```styx
/// build_steps {
///   styx_to_json {
///     params {
///       file @file
///     }
///     command (styx --json "{file}")
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, Facet)]
#[facet(rename_all = "snake_case")]
pub struct BuildStepDef {
    /// Typed parameters for this build step.
    /// Keys are parameter names, values are Styx schema types.
    /// Use `@file` for file paths that should be tracked for caching.
    #[facet(default)]
    pub params: Option<HashMap<String, Schema>>,

    /// Command to execute as a sequence of arguments.
    /// Use `{param_name}` for interpolation.
    /// If absent, the step reads the file specified by the first `@file` param.
    #[facet(default)]
    pub command: Option<Vec<String>>,
}

impl BuildStepDef {
    /// Check if a parameter is a tracked file type.
    pub fn is_file_param(&self, param_name: &str) -> bool {
        self.params
            .as_ref()
            .and_then(|p| p.get(param_name))
            .map(|schema| matches!(schema, Schema::Type { name: Some(n) } if n == "file"))
            .unwrap_or(false)
    }

    /// Get all file-typed parameter names.
    pub fn file_params(&self) -> Vec<&str> {
        self.params
            .as_ref()
            .map(|p| {
                p.iter()
                    .filter(|(_, schema)| {
                        matches!(schema, Schema::Type { name: Some(n) } if n == "file")
                    })
                    .map(|(name, _)| name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}
