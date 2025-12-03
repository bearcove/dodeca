//! SASS/SCSS compilation plugin for dodeca
//!
//! Compiles SASS/SCSS to CSS using grass.

use facet::Facet;
use plugcard::{plugcard, PlugResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

plugcard::export_plugin!();

/// Input for SASS compilation - a map of filename -> content pairs
#[derive(Facet)]
pub struct SassInput {
    pub files: HashMap<String, String>,
}

/// Compile SASS/SCSS to CSS
///
/// Takes a map of filename -> content pairs, with "main.scss" as the entry point.
/// Returns compiled CSS, or an error if compilation fails.
#[plugcard]
pub fn compile_sass(input: SassInput) -> PlugResult<String> {
    let files = input.files;

    // Find main.scss
    let main_content = match files.get("main.scss") {
        Some(content) => content,
        None => return PlugResult::Err("main.scss not found in files".to_string()),
    };

    // Create an in-memory filesystem for grass
    let fs = InMemorySassFs::new(&files);

    // Compile with grass using in-memory fs
    let options = grass::Options::default().fs(&fs);

    match grass::from_string(main_content.clone(), &options) {
        Ok(css) => PlugResult::Ok(css),
        Err(e) => PlugResult::Err(format!("SASS compilation failed: {}", e)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_simple_sass() {
        let mut files = HashMap::new();
        files.insert(
            "main.scss".to_string(),
            r#"
            $primary: #ff0000;
            body {
                color: $primary;
            }
            "#
            .to_string(),
        );

        let result = compile_sass(SassInput { files });
        let PlugResult::Ok(css) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(css.contains("color:") || css.contains("color :"));
        assert!(css.contains("#ff0000") || css.contains("red"));
    }

    #[test]
    fn test_compile_with_import() {
        let mut files = HashMap::new();
        files.insert(
            "_variables.scss".to_string(),
            "$primary: blue;".to_string(),
        );
        files.insert(
            "main.scss".to_string(),
            r#"
            @use "variables";
            body {
                color: variables.$primary;
            }
            "#
            .to_string(),
        );

        let result = compile_sass(SassInput { files });
        let PlugResult::Ok(css) = result else {
            panic!("Expected Ok, got {:?}", result);
        };
        assert!(css.contains("color:") || css.contains("color :"));
    }

    #[test]
    fn test_missing_main() {
        let files = HashMap::new();
        let result = compile_sass(SassInput { files });
        assert!(matches!(result, PlugResult::Err(_)));
    }
}
