//! Dodeca SASS cell (cell-sass)
//!
//! This cell handles SASS/SCSS compilation using grass.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cell_sass_proto::{SassCompiler, SassCompilerServer, SassInput, SassResult};

/// SASS compiler implementation
pub struct SassCompilerImpl;

impl SassCompiler for SassCompilerImpl {
    async fn compile_sass(&self, input: SassInput) -> SassResult {
        let files = input.files;

        // Find main.scss
        let main_content = match files.get("main.scss") {
            Some(content) => content,
            None => {
                return SassResult::Error {
                    message: "main.scss not found in files".to_string(),
                };
            }
        };

        // Create an in-memory filesystem for grass
        let fs = InMemorySassFs::new(&files);

        // Compile with grass using in-memory fs
        let options = grass::Options::default().fs(&fs);

        match grass::from_string(main_content.clone(), &options) {
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
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.files.get(path).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {path:?}"),
            )
        })
    }
}

rapace_cell::cell_service!(SassCompilerServer<SassCompilerImpl>, SassCompilerImpl);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(SassCompilerImpl)).await?;
    Ok(())
}
