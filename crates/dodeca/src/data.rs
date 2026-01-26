//! Data file loading and parsing for template variables.
//!
//! Supports JSON, TOML, and YAML data files. Files are loaded from
//! the `data/` directory (sibling to content/) and exposed in templates
//! under the `data` namespace.
//!
//! Parsing is delegated to the data cell to reduce monomorphization in
//! the main binary.
//!
//! # Example
//!
//! Given `data/versions.toml`:
//! ```toml
//! [dodeca]
//! version = "0.1.0"
//! ```
//!
//! In templates:
//! ```jinja
//! {{ data.versions.dodeca.version }}
//! ```

use crate::cells;
use crate::db::DataFile;
pub use cell_data_proto::DataFormat;
use cell_data_proto::LoadDataResult;
use facet_value::{VObject, VString, Value};

/// Parse a data file into a template Value via the data cell.
pub async fn parse_data_file(content: &str, format: DataFormat) -> Result<Value, String> {
    let client = cells::data_cell()
        .await
        .ok_or_else(|| "Data cell not available".to_string())?;

    match client.load_data(content.to_string(), format).await {
        Ok(LoadDataResult::Success { value }) => Ok(value),
        Ok(LoadDataResult::Error { message }) => Err(message),
        Err(e) => Err(format!("RPC error: {:?}", e)),
    }
}

/// Parse raw data files (path, content) and merge into a single Value object.
/// Each file becomes a key in the object (filename without extension).
pub async fn parse_raw_data_files(files: &[(String, String)]) -> Value {
    let mut data_map = VObject::new();

    for (path, content) in files {
        // Get filename without extension as the key
        let key = if let Some(dot_pos) = path.rsplit('/').next().unwrap_or(path).rfind('.') {
            &path.rsplit('/').next().unwrap_or(path)[..dot_pos]
        } else {
            path.rsplit('/').next().unwrap_or(path)
        };

        let Some(format) = DataFormat::from_extension(path) else {
            tracing::warn!("Unknown data file format: {}", path);
            continue;
        };

        match parse_data_file(content, format).await {
            Ok(value) => {
                data_map.insert(VString::from(key), value);
            }
            Err(e) => {
                tracing::warn!("Failed to parse data file {}: {}", path, e);
            }
        }
    }

    data_map.into()
}

/// Load all data files and merge them into a single Value object.
/// Each file becomes a key in the object (filename without extension).
#[allow(dead_code)]
pub async fn load_data_files(db: &crate::db::Database, data_files: &[DataFile]) -> Value {
    let mut data_map = VObject::new();

    for file in data_files {
        let Ok(path) = file.path(db) else { continue };
        let Ok(content) = file.content(db) else {
            continue;
        };
        let path = path.as_str();
        let content = content.as_str();

        // Get filename without extension as the key
        let key = if let Some(dot_pos) = path.rsplit('/').next().unwrap_or(path).rfind('.') {
            &path.rsplit('/').next().unwrap_or(path)[..dot_pos]
        } else {
            path.rsplit('/').next().unwrap_or(path)
        };

        let Some(format) = DataFormat::from_extension(path) else {
            tracing::warn!("Unknown data file format: {}", path);
            continue;
        };

        match parse_data_file(content, format).await {
            Ok(value) => {
                data_map.insert(VString::from(key), value);
            }
            Err(e) => {
                tracing::warn!("Failed to parse data file {}: {}", path, e);
            }
        }
    }

    data_map.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_extension() {
        assert_eq!(
            DataFormat::from_extension("foo.toml"),
            Some(DataFormat::Toml)
        );
        assert_eq!(
            DataFormat::from_extension("bar.json"),
            Some(DataFormat::Json)
        );
        assert_eq!(
            DataFormat::from_extension("qux.yaml"),
            Some(DataFormat::Yaml)
        );
        assert_eq!(
            DataFormat::from_extension("qux.yml"),
            Some(DataFormat::Yaml)
        );
        assert_eq!(DataFormat::from_extension("unknown.txt"), None);
        assert_eq!(DataFormat::from_extension("old.kdl"), None); // KDL no longer supported
    }
}
