//! Shared configuration types for dodeca code execution
//!
//! This crate contains types that are shared between:
//! - The main dodeca binary (for KDL config parsing)
//! - The dodeca-code-execution plugin (for runtime execution)

use facet::Facet;
use facet_kdl as kdl;

/// Code execution configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct CodeExecutionConfig {
    /// Enable/disable code execution
    #[facet(kdl::property, default)]
    pub enabled: Option<bool>,

    /// Fail build on execution errors in dev mode
    #[facet(kdl::property, default)]
    pub fail_on_error: Option<bool>,

    /// Execution timeout in seconds
    #[facet(kdl::property, default)]
    pub timeout_secs: Option<u64>,

    /// Cache directory for execution artifacts
    #[facet(kdl::property, default)]
    pub cache_dir: Option<String>,

    /// Dependencies for code samples
    #[facet(kdl::child, default)]
    pub dependencies: DependenciesConfig,

    /// Language-specific configuration
    #[facet(kdl::child, default)]
    pub rust: RustConfig,
}

/// Dependencies configuration
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct DependenciesConfig {
    /// List of dependency specifications
    #[facet(kdl::children, default)]
    pub deps: Vec<DependencySpec>,
}

/// A single dependency specification
///
/// Supports crates.io, git, and path dependencies:
///
/// ```kdl
/// dependencies {
///     // crates.io dependency
///     serde "1.0"
///
///     // crates.io with features
///     serde "1.0" features=["derive"]
///
///     // git dependency with branch
///     facet "0.1" git="https://github.com/facet-rs/facet" branch="main"
///
///     // git dependency with rev
///     facet "0.1" git="https://github.com/facet-rs/facet" rev="abc123"
///
///     // path dependency (relative to project root)
///     plugcard "0.1" path="crates/plugcard"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
pub struct DependencySpec {
    /// Crate name (from KDL node name)
    #[facet(kdl::node_name)]
    pub name: String,

    /// Version requirement (positional argument)
    #[facet(kdl::argument)]
    pub version: String,

    /// Git repository URL (optional)
    #[facet(kdl::property, default)]
    pub git: Option<String>,

    /// Git revision/commit hash (optional)
    #[facet(kdl::property, default)]
    pub rev: Option<String>,

    /// Git branch (optional)
    #[facet(kdl::property, default)]
    pub branch: Option<String>,

    /// Local path (optional, relative to project root)
    #[facet(kdl::property, default)]
    pub path: Option<String>,

    /// Crate features to enable (optional)
    #[facet(kdl::property, default)]
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
    ///
    /// If `project_root` is provided, path dependencies will be made absolute.
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

/// Wrapper for single-argument child nodes (e.g., `command "cargo"`)
#[derive(Debug, Clone, Facet)]
pub struct SingleValue<T: 'static> {
    #[facet(kdl::argument)]
    pub value: T,
}

/// Wrapper for multi-argument child nodes (e.g., `args "run" "--quiet" "--release"`)
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct MultiValue<T: Default + 'static> {
    #[facet(kdl::arguments, default)]
    pub values: Vec<T>,
}

/// Rust-specific configuration
///
/// All fields are child nodes with arguments (not properties).
/// Example KDL:
/// ```kdl
/// rust {
///     command "cargo"
///     args "run" "--quiet" "--release"
///     extension "rs"
///     prepare_code #true
///     auto_imports "use std::*;"
///     show_output #true
/// }
/// ```
#[derive(Debug, Clone, Default, Facet)]
#[facet(traits(Default))]
pub struct RustConfig {
    /// Cargo command: `command "cargo"`
    #[facet(kdl::child, default)]
    pub command: Option<SingleValue<String>>,

    /// Cargo arguments: `args "run" "--quiet" "--release"`
    #[facet(kdl::child, default)]
    pub args: Option<MultiValue<String>>,

    /// File extension: `extension "rs"`
    #[facet(kdl::child, default)]
    pub extension: Option<SingleValue<String>>,

    /// Auto-wrap code without main function: `prepare_code #true`
    #[facet(kdl::child, default)]
    pub prepare_code: Option<SingleValue<bool>>,

    /// Auto-imports: `auto_imports "use std::*;" "use facet::*;"`
    #[facet(kdl::child, default)]
    pub auto_imports: Option<MultiValue<String>>,

    /// Show output in build: `show_output #true`
    #[facet(kdl::child, default)]
    pub show_output: Option<SingleValue<bool>>,
}

/// Default dependencies for Rust code samples
pub fn default_rust_dependencies() -> Vec<DependencySpec> {
    vec![
        DependencySpec::git_branch("facet", "https://github.com/facet-rs/facet", "main"),
        DependencySpec::git_branch("facet-json", "https://github.com/facet-rs/facet", "main"),
        DependencySpec::git_branch("facet-kdl", "https://github.com/facet-rs/facet", "main"),
    ]
}

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
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "facet");
        assert_eq!(deps[1].name, "facet-json");
        assert_eq!(deps[2].name, "facet-kdl");
    }

    /// Wrapper to test RustConfig as it appears in actual KDL files
    #[derive(Debug, Facet)]
    struct RustConfigWrapper {
        #[facet(kdl::child)]
        rust: RustConfig,
    }

    #[test]
    fn test_rust_config_child_nodes() {
        // This KDL matches the rapace config format where all fields
        // are child nodes with positional arguments
        let kdl = r#"
rust {
    command "cargo"
    args "run" "--quiet" "--release"
    extension "rs"
    prepare_code #true
    auto_imports "use std::collections::HashMap;"
    show_output #true
}
"#;

        let wrapper: RustConfigWrapper = facet_kdl::from_str(kdl).unwrap();
        let config = wrapper.rust;

        // Single-value child nodes
        assert_eq!(
            config.command.as_ref().map(|c| c.value.as_str()),
            Some("cargo")
        );
        assert_eq!(
            config.extension.as_ref().map(|e| e.value.as_str()),
            Some("rs")
        );
        assert_eq!(config.prepare_code.as_ref().map(|p| p.value), Some(true));
        assert_eq!(config.show_output.as_ref().map(|s| s.value), Some(true));

        // Multi-value child nodes
        let args = config.args.expect("args should be present");
        assert_eq!(args.values, vec!["run", "--quiet", "--release"]);

        let imports = config.auto_imports.expect("auto_imports should be present");
        assert_eq!(imports.values, vec!["use std::collections::HashMap;"]);
    }

    #[test]
    fn test_rust_config_empty_children() {
        // Config without optional children should still work
        let kdl = r#"
rust {
    command "cargo"
    extension "rs"
}
"#;

        let wrapper: RustConfigWrapper = facet_kdl::from_str(kdl).unwrap();
        let config = wrapper.rust;
        assert_eq!(
            config.command.as_ref().map(|c| c.value.as_str()),
            Some("cargo")
        );
        assert_eq!(
            config.extension.as_ref().map(|e| e.value.as_str()),
            Some("rs")
        );
        assert!(config.args.is_none());
        assert!(config.auto_imports.is_none());
    }
}
