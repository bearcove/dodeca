//! Data file loading and parsing for template variables.
//!
//! Supports JSON, TOML, and YAML data files. Files are loaded from
//! the `data/` directory (sibling to content/) and exposed in templates
//! under the `data` namespace.
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

use crate::db::DataFile;
use facet_format::{DynDeserializeError, DynParser, FormatDeserializer};
use facet_value::{VObject, VString, Value};

/// Supported data file formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Json,
    Toml,
    Yaml,
}

impl DataFormat {
    /// Determine format from file extension
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next()?.to_lowercase();
        match ext.as_str() {
            "json" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

/// Parse a data file into a template Value using dynamic dispatch.
///
/// This uses `dyn DynParser` to share a single monomorphization of the
/// deserializer across all format types, reducing binary size.
pub fn parse_data_file(content: &str, format: DataFormat) -> Result<Value, String> {
    match format {
        DataFormat::Json => {
            let mut parser = facet_json::JsonParser::new(content.as_bytes());
            deserialize_value(&mut parser).map_err(|e| format!("JSON parse error: {e}"))
        }
        DataFormat::Toml => {
            let mut parser = facet_toml::TomlParser::new(content)
                .map_err(|e| format!("TOML parse error: {e}"))?;
            deserialize_value(&mut parser).map_err(|e| format!("TOML parse error: {e}"))
        }
        DataFormat::Yaml => {
            let mut parser = facet_yaml::YamlParser::new(content)
                .map_err(|e| format!("YAML parse error: {e}"))?;
            deserialize_value(&mut parser).map_err(|e| format!("YAML parse error: {e}"))
        }
    }
}

/// Deserialize a Value using dynamic dispatch.
///
/// This function only has one monomorphization regardless of parser type,
/// reducing code bloat when multiple formats are used.
fn deserialize_value(parser: &mut dyn DynParser<'_>) -> Result<Value, DynDeserializeError> {
    let mut de = FormatDeserializer::new(parser);
    de.deserialize()
}

/// Parse raw data files (path, content) and merge into a single Value object
/// Each file becomes a key in the object (filename without extension)
pub fn parse_raw_data_files(files: &[(String, String)]) -> Value {
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

        match parse_data_file(content, format) {
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

/// Load all data files and merge them into a single Value object
/// Each file becomes a key in the object (filename without extension)
#[allow(dead_code)]
pub fn load_data_files(db: &crate::db::Database, data_files: &[DataFile]) -> Value {
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

        match parse_data_file(content, format) {
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
    use facet_value::DestructuredRef;

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

    #[test]
    fn test_parse_toml() {
        let content = r#"
[project]
name = "dodeca"
version = "0.1.0"
"#;
        let value = parse_data_file(content, DataFormat::Toml).unwrap();
        if let DestructuredRef::Object(map) = value.destructure_ref() {
            if let Some(project) = map.get("project") {
                if let DestructuredRef::Object(project_map) = project.destructure_ref() {
                    if let Some(name) = project_map.get("name") {
                        if let DestructuredRef::String(s) = name.destructure_ref() {
                            assert_eq!(s.as_str(), "dodeca");
                        } else {
                            panic!("Expected name to be a string");
                        }
                    } else {
                        panic!("Expected name field");
                    }
                    if let Some(version) = project_map.get("version") {
                        if let DestructuredRef::String(s) = version.destructure_ref() {
                            assert_eq!(s.as_str(), "0.1.0");
                        } else {
                            panic!("Expected version to be a string");
                        }
                    } else {
                        panic!("Expected version field");
                    }
                } else {
                    panic!("Expected project to be an object");
                }
            } else {
                panic!("Expected project field");
            }
        } else {
            panic!("Expected object");
        }
    }

    #[test]
    fn test_parse_json() {
        let content = r#"{"name": "test", "count": 42}"#;
        let value = parse_data_file(content, DataFormat::Json).unwrap();
        if let DestructuredRef::Object(map) = value.destructure_ref() {
            if let Some(name) = map.get("name") {
                if let DestructuredRef::String(s) = name.destructure_ref() {
                    assert_eq!(s.as_str(), "test");
                } else {
                    panic!("Expected name to be a string");
                }
            } else {
                panic!("Expected name field");
            }
            if let Some(count) = map.get("count") {
                if let DestructuredRef::Number(n) = count.destructure_ref() {
                    assert_eq!(n.to_i64(), Some(42));
                } else {
                    panic!("Expected count to be a number");
                }
            } else {
                panic!("Expected count field");
            }
        } else {
            panic!("Expected object");
        }
    }

    #[test]
    fn test_parse_yaml() {
        let content = r#"
name: test
items:
  - one
  - two
"#;
        let value = parse_data_file(content, DataFormat::Yaml).unwrap();
        if let DestructuredRef::Object(map) = value.destructure_ref() {
            if let Some(name) = map.get("name") {
                if let DestructuredRef::String(s) = name.destructure_ref() {
                    assert_eq!(s.as_str(), "test");
                } else {
                    panic!("Expected name to be a string");
                }
            } else {
                panic!("Expected name field");
            }
            if let Some(items) = map.get("items") {
                if let DestructuredRef::Array(arr) = items.destructure_ref() {
                    assert_eq!(arr.len(), 2);
                    if let DestructuredRef::String(s) = arr.get(0).unwrap().destructure_ref() {
                        assert_eq!(s.as_str(), "one");
                    } else {
                        panic!("Expected first item to be a string");
                    }
                } else {
                    panic!("Expected items to be an array");
                }
            } else {
                panic!("Expected items field");
            }
        } else {
            panic!("Expected object");
        }
    }
}
