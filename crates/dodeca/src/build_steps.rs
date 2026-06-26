//! Build step execution with file-based caching.
//!
//! Build steps are parameterized commands defined in config and invoked from templates.
//! Results are cached based on step name, parameter values, and file content hashes.

use std::collections::HashMap;
use std::hash::Hasher;
use std::process::Stdio;

use camino::{Utf8Path, Utf8PathBuf};
use dashmap::DashMap;
use dodeca_config::BuildStepDef;
use rapidhash::fast::RapidHasher;
use tokio::process::Command;

/// Cache key for a build step invocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    /// Mount of the source whose steps were invoked (steps are source-scoped, so
    /// two sources may define a same-named step with different commands).
    mount: String,
    /// Name of the build step
    step_name: String,
    /// Parameter values (sorted for consistency)
    params: Vec<(String, String)>,
    /// Hashes of file-typed parameters
    file_hashes: Vec<(String, u64)>,
}

/// Result of a build step execution.
#[derive(Debug, Clone)]
pub enum BuildStepResult {
    /// Successful execution with captured stdout
    Success(Vec<u8>),
    /// Execution failed with error message
    Error(String),
}

/// One source's build steps and the directory they run in.
#[derive(Debug, Clone)]
struct SourceSteps {
    steps: HashMap<String, BuildStepDef>,
    project_dir: Utf8PathBuf,
}

/// Executor for build steps with caching. Build steps are source-scoped: each
/// content source contributes its own steps, which run in that source's project
/// dir. A `build("step")` call resolves against the *rendering* source's bucket
/// (keyed by mount), so a mounted sub-repo's `git_hash` reflects *its* HEAD.
pub struct BuildStepExecutor {
    /// Per-source steps, keyed by normalized mount (`/`, `/wiki/`, …).
    by_mount: HashMap<String, SourceSteps>,
    /// Cache of execution results (keyed by mount + step + params).
    cache: DashMap<CacheKey, BuildStepResult>,
}

impl BuildStepExecutor {
    /// Build an executor from every source's steps + project dir.
    pub fn new(sources: &[crate::config::ResolvedSource]) -> Self {
        let by_mount: HashMap<String, SourceSteps> = sources
            .iter()
            .map(|s| {
                (
                    s.mount.clone(),
                    SourceSteps {
                        steps: s.build_steps.clone(),
                        project_dir: s.project_dir.clone(),
                    },
                )
            })
            .collect();
        tracing::debug!(
            sources = by_mount.len(),
            total_steps = by_mount.values().map(|s| s.steps.len()).sum::<usize>(),
            "BuildStepExecutor initialized"
        );
        Self {
            by_mount,
            cache: DashMap::new(),
        }
    }

    /// Clear the cache (call at the start of each build).
    #[allow(dead_code)]
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Execute a build step (for the source mounted at `mount`) with the given
    /// parameters.
    pub async fn execute(
        &self,
        mount: &str,
        step_name: &str,
        params: &HashMap<String, String>,
    ) -> BuildStepResult {
        tracing::debug!(mount, step_name, ?params, "executing build step");

        let Some(source) = self.by_mount.get(mount) else {
            return BuildStepResult::Error(format!("No source mounted at '{}'", mount));
        };
        let project_root = &source.project_dir;

        // Look up the step definition
        let step_def = match source.steps.get(step_name) {
            Some(def) => def,
            None => {
                return BuildStepResult::Error(format!("Unknown build step: {}", step_name));
            }
        };

        // Validate parameters
        if let Some(expected_params) = &step_def.params {
            for param_name in expected_params.keys() {
                if !params.contains_key(param_name) {
                    return BuildStepResult::Error(format!(
                        "Missing parameter '{}' for build step '{}'",
                        param_name, step_name
                    ));
                }
            }
        }

        // Build cache key
        let cache_key = match self
            .build_cache_key(mount, project_root, step_name, step_def, params)
            .await
        {
            Ok(key) => key,
            Err(e) => return BuildStepResult::Error(e),
        };

        // Check cache
        if let Some(cached) = self.cache.get(&cache_key) {
            tracing::debug!(step = %step_name, "Build step cache hit");
            return cached.clone();
        }

        // Execute the step
        tracing::info!(step = %step_name, "Executing build step");
        let result = self
            .execute_inner(project_root, step_name, step_def, params)
            .await;
        tracing::info!(step = %step_name, ?result, "Build step result");

        // Cache the result
        self.cache.insert(cache_key, result.clone());

        result
    }

    /// Build the cache key for a step invocation.
    async fn build_cache_key(
        &self,
        mount: &str,
        project_root: &Utf8Path,
        step_name: &str,
        step_def: &BuildStepDef,
        params: &HashMap<String, String>,
    ) -> Result<CacheKey, String> {
        let mut sorted_params: Vec<(String, String)> =
            params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        sorted_params.sort_by(|a, b| a.0.cmp(&b.0));

        // Hash file-typed parameters
        let mut file_hashes = Vec::new();
        for param_name in step_def.file_params() {
            if let Some(file_path) = params.get(param_name) {
                let full_path = project_root.join(file_path);
                let hash = hash_file(&full_path).await.map_err(|e| {
                    format!(
                        "Failed to hash file '{}' for parameter '{}': {}",
                        file_path, param_name, e
                    )
                })?;
                file_hashes.push((param_name.to_string(), hash));
            }
        }
        file_hashes.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(CacheKey {
            mount: mount.to_string(),
            step_name: step_name.to_string(),
            params: sorted_params,
            file_hashes,
        })
    }

    /// Execute the build step (no caching).
    async fn execute_inner(
        &self,
        project_root: &Utf8Path,
        step_name: &str,
        step_def: &BuildStepDef,
        params: &HashMap<String, String>,
    ) -> BuildStepResult {
        match &step_def.command {
            Some(cmd_args) => {
                // Execute command
                self.execute_command(project_root, step_name, cmd_args, params)
                    .await
            }
            None => {
                // No command = read file from first @file param
                self.read_file_param(project_root, step_name, step_def, params)
                    .await
            }
        }
    }

    /// Execute a command with parameter interpolation.
    async fn execute_command(
        &self,
        project_root: &Utf8Path,
        step_name: &str,
        cmd_args: &[String],
        params: &HashMap<String, String>,
    ) -> BuildStepResult {
        if cmd_args.is_empty() {
            return BuildStepResult::Error(format!("Build step '{}' has empty command", step_name));
        }

        // Interpolate parameters into command arguments
        let interpolated: Vec<String> = cmd_args
            .iter()
            .map(|arg| interpolate_params(arg, params))
            .collect();

        let program = &interpolated[0];
        let args = &interpolated[1..];

        tracing::info!(
            step = %step_name,
            program = %program,
            args = ?args,
            "Executing build step"
        );

        // Execute the command
        let output = match Command::new(program)
            .args(args)
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(output) => output,
            Err(e) => {
                return BuildStepResult::Error(format!("Failed to execute '{}': {}", program, e));
            }
        };

        if output.status.success() {
            BuildStepResult::Success(output.stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            BuildStepResult::Error(format!(
                "Command failed with exit code {:?}: {}",
                output.status.code(),
                stderr
            ))
        }
    }

    /// Read file content when no command is specified.
    async fn read_file_param(
        &self,
        project_root: &Utf8Path,
        step_name: &str,
        step_def: &BuildStepDef,
        params: &HashMap<String, String>,
    ) -> BuildStepResult {
        // Find the first @file parameter
        let file_params = step_def.file_params();
        let param_name = match file_params.first() {
            Some(name) => *name,
            None => {
                return BuildStepResult::Error(format!(
                    "Build step '{}' has no command and no @file parameter",
                    step_name
                ));
            }
        };

        let file_path = match params.get(param_name) {
            Some(path) => path,
            None => {
                return BuildStepResult::Error(format!(
                    "Missing parameter '{}' for build step '{}'",
                    param_name, step_name
                ));
            }
        };

        let full_path = project_root.join(file_path);
        match tokio::fs::read(&full_path).await {
            Ok(contents) => BuildStepResult::Success(contents),
            Err(e) => BuildStepResult::Error(format!("Failed to read file '{}': {}", full_path, e)),
        }
    }
}

/// Interpolate `{param}` placeholders in a string.
fn interpolate_params(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in params {
        result = result.replace(&format!("{{{}}}", key), value);
    }
    result
}

/// Hash a file's contents using rapidhash.
async fn hash_file(path: &Utf8Path) -> std::io::Result<u64> {
    let contents = tokio::fs::read(path).await?;
    let mut hasher = RapidHasher::default();
    hasher.write(&contents);
    Ok(hasher.finish())
}

/// Built-in `read` function that reads a file.
pub async fn builtin_read(project_root: &Utf8Path, file_path: &str) -> BuildStepResult {
    let full_path = project_root.join(file_path);
    match tokio::fs::read(&full_path).await {
        Ok(contents) => BuildStepResult::Success(contents),
        Err(e) => BuildStepResult::Error(format!("Failed to read file '{}': {}", full_path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_params() {
        let mut params = HashMap::new();
        params.insert("file".to_string(), "test.txt".to_string());
        params.insert("width".to_string(), "100".to_string());

        assert_eq!(interpolate_params("{file}", &params), "test.txt");
        assert_eq!(
            interpolate_params("convert {file} -resize {width}x", &params),
            "convert test.txt -resize 100x"
        );
        assert_eq!(
            interpolate_params("no params here", &params),
            "no params here"
        );
    }
}
