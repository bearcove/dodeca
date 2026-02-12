//! Dodeca SASS cell (cell-sass)
//!
//! This cell handles SASS/SCSS compilation using grass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dodeca_cell_runtime::run_cell;

use cell_sass_proto::{SassCompiler, SassCompilerDispatcher, SassInput, SassResult};

/// SASS compiler implementation
#[derive(Clone)]
pub struct SassCompilerImpl;

impl SassCompiler for SassCompilerImpl {
    async fn compile_sass(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        input: SassInput,
    ) -> SassResult {
        let files = input.files;

        // Find main.scss
        if !files.contains_key("main.scss") {
            return SassResult::Error {
                message: "main.scss not found in files".to_string(),
            };
        }

        // Create an in-memory filesystem for grass
        let fs = InMemorySassFs::new(&files);

        // Compile with grass using in-memory fs plus optional disk-backed load paths
        let load_paths: Vec<PathBuf> = input.load_paths.iter().map(PathBuf::from).collect();
        let options = grass::Options::default().fs(&fs).load_paths(&load_paths);

        match grass::from_path("main.scss", &options) {
            Ok(css) => SassResult::Success { css },
            Err(e) => SassResult::Error {
                message: format!("SASS compilation failed: {}", e),
            },
        }
    }
}

/// In-memory filesystem for grass SASS compiler
#[derive(Debug)]
struct InMemorySassFs {
    files: HashMap<PathBuf, Vec<u8>>,
}

impl InMemorySassFs {
    fn new(sass_map: &HashMap<String, String>) -> Self {
        let files = sass_map
            .iter()
            .map(|(path, content)| (PathBuf::from(path), content.as_bytes().to_vec()))
            .collect();
        Self { files }
    }
}

impl grass::Fs for InMemorySassFs {
    fn is_dir(&self, path: &Path) -> bool {
        // Check if any file is under this directory
        self.files.keys().any(|f| f.starts_with(path))
            || std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
            || std::fs::metadata(path)
                .map(|m| m.is_file())
                .unwrap_or(false)
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        if let Some(content) = self.files.get(path) {
            return Ok(content.clone());
        }

        std::fs::read(path)
            .map_err(|e| std::io::Error::new(e.kind(), format!("File not found: {path:?}: {e}")))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("sass", |_handle| SassCompilerDispatcher::new(
        SassCompilerImpl
    ))
}
