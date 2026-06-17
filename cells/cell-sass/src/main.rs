//! Dodeca SASS processor.
//!
//! This processor handles SASS/SCSS compilation using grass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cell_sass_proto::{SassCompiler, SassResult};

/// SASS compiler implementation
#[derive(Clone)]
pub struct SassCompilerImpl;

impl SassCompiler for SassCompilerImpl {
    async fn compile_sass(
        &self,
        entrypoint: String,
        files: HashMap<String, String>,
        load_paths: Vec<String>,
    ) -> SassResult {
        if !files.contains_key(&entrypoint) {
            return SassResult::Error {
                message: format!("{entrypoint} not found in files"),
            };
        }

        // Create an in-memory filesystem for grass
        let fs = InMemorySassFs::new(&files);

        let load_paths: Vec<PathBuf> = load_paths.into_iter().map(PathBuf::from).collect();
        let options = grass::Options::default().fs(&fs).load_paths(&load_paths);

        match grass::from_path(entrypoint, &options) {
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
