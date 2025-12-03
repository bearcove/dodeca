//! Data file loading and parsing for template variables.
//!
//! Supports KDL, JSON, TOML, and YAML data files. Files are loaded from
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
use crate::template::Value;
use std::collections::HashMap;

/// Supported data file formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Kdl,
    Json,
    Toml,
    Yaml,
}

impl DataFormat {
    /// Determine format from file extension
    pub fn from_extension(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next()?.to_lowercase();
        match ext.as_str() {
            "kdl" => Some(Self::Kdl),
            "json" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "yaml" | "yml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

/// Parse a data file into a template Value
pub fn parse_data_file(content: &str, format: DataFormat) -> Result<Value, String> {
    match format {
        DataFormat::Kdl => parse_kdl(content),
        DataFormat::Json => parse_json(content),
        DataFormat::Toml => parse_toml(content),
        DataFormat::Yaml => parse_yaml(content),
    }
}

fn parse_kdl(content: &str) -> Result<Value, String> {
    let value: facet_value::Value =
        facet_kdl::from_str(content).map_err(|e| format!("KDL parse error: {e}"))?;
    Ok(facet_value_to_template_value(value))
}

fn parse_json(content: &str) -> Result<Value, String> {
    let value: facet_value::Value =
        facet_json::from_str(content).map_err(|e| format!("JSON parse error: {e}"))?;
    Ok(facet_value_to_template_value(value))
}

fn parse_toml(content: &str) -> Result<Value, String> {
    let value: facet_value::Value =
        facet_toml::from_str(content).map_err(|e| format!("TOML parse error: {e}"))?;
    Ok(facet_value_to_template_value(value))
}

fn parse_yaml(content: &str) -> Result<Value, String> {
    // Use serde_yaml because facet-yaml doesn't support dynamic values
    let serde_value: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|e| format!("YAML parse error: {e}"))?;
    Ok(serde_value_to_template_value(serde_value))
}

/// Convert a serde_yaml::Value to a template Value
fn serde_value_to_template_value(v: serde_yaml::Value) -> Value {
    match v {
        serde_yaml::Value::Null => Value::None,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Int(0)
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(arr) => {
            let items: Vec<Value> = arr.into_iter().map(serde_value_to_template_value).collect();
            Value::List(items)
        }
        serde_yaml::Value::Mapping(map) => {
            let mut result = HashMap::new();
            for (key, val) in map {
                if let serde_yaml::Value::String(k) = key {
                    result.insert(k, serde_value_to_template_value(val));
                }
            }
            Value::Dict(result)
        }
        serde_yaml::Value::Tagged(tagged) => {
            // Unwrap tagged values
            serde_value_to_template_value(tagged.value)
        }
    }
}

/// Convert a facet_value::Value to a template Value
fn facet_value_to_template_value(v: facet_value::Value) -> Value {
    use facet_value::ValueType;

    match v.value_type() {
        ValueType::Null => Value::None,
        ValueType::Bool => Value::Bool(v.as_bool().unwrap_or(false)),
        ValueType::Number => {
            if let Some(num) = v.as_number() {
                // Try integer first, then float
                if let Some(i) = num.to_i64() {
                    Value::Int(i)
                } else if let Some(f) = num.to_f64() {
                    Value::Float(f)
                } else {
                    Value::Int(0)
                }
            } else {
                Value::Int(0)
            }
        }
        ValueType::String => {
            if let Some(s) = v.as_string() {
                Value::String(s.as_str().to_string())
            } else {
                Value::String(String::new())
            }
        }
        ValueType::Bytes => {
            if let Some(b) = v.as_bytes() {
                // Convert bytes to base64 string
                use base64::Engine;
                Value::String(base64::engine::general_purpose::STANDARD.encode(b.as_slice()))
            } else {
                Value::String(String::new())
            }
        }
        ValueType::Array => {
            if let Some(arr) = v.as_array() {
                let items: Vec<Value> = arr
                    .iter()
                    .map(|item| facet_value_to_template_value(item.clone()))
                    .collect();
                Value::List(items)
            } else {
                Value::List(vec![])
            }
        }
        ValueType::Object => {
            if let Some(obj) = v.as_object() {
                let mut map = HashMap::new();
                for (key, val) in obj.iter() {
                    map.insert(key.to_string(), facet_value_to_template_value(val.clone()));
                }
                Value::Dict(map)
            } else {
                Value::Dict(HashMap::new())
            }
        }
        ValueType::DateTime => {
            // Convert datetime to string representation
            if let Some(dt) = v.as_datetime() {
                Value::String(format!("{:?}", dt))
            } else {
                Value::None
            }
        }
    }
}

/// Parse raw data files (path, content) and merge into a single Value::Dict
/// Each file becomes a key in the dict (filename without extension)
pub fn parse_raw_data_files(files: &[(String, String)]) -> Value {
    let mut data_map = HashMap::new();

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
                data_map.insert(key.to_string(), value);
            }
            Err(e) => {
                tracing::warn!("Failed to parse data file {}: {}", path, e);
            }
        }
    }

    Value::Dict(data_map)
}

/// Load all data files and merge them into a single Value::Dict
/// Each file becomes a key in the dict (filename without extension)
#[allow(dead_code)]
pub fn load_data_files(db: &dyn crate::db::Db, data_files: &[DataFile]) -> Value {
    let mut data_map = HashMap::new();

    for file in data_files {
        let path = file.path(db).as_str();
        let content = file.content(db).as_str();

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
                data_map.insert(key.to_string(), value);
            }
            Err(e) => {
                tracing::warn!("Failed to parse data file {}: {}", path, e);
            }
        }
    }

    Value::Dict(data_map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_extension() {
        assert_eq!(DataFormat::from_extension("foo.toml"), Some(DataFormat::Toml));
        assert_eq!(DataFormat::from_extension("bar.json"), Some(DataFormat::Json));
        assert_eq!(DataFormat::from_extension("baz.kdl"), Some(DataFormat::Kdl));
        assert_eq!(DataFormat::from_extension("qux.yaml"), Some(DataFormat::Yaml));
        assert_eq!(DataFormat::from_extension("qux.yml"), Some(DataFormat::Yaml));
        assert_eq!(DataFormat::from_extension("unknown.txt"), None);
    }

    #[test]
    fn test_parse_toml() {
        let content = r#"
[project]
name = "dodeca"
version = "0.1.0"
"#;
        let value = parse_toml(content).unwrap();
        if let Value::Dict(map) = value {
            if let Some(Value::Dict(project)) = map.get("project") {
                match project.get("name") {
                    Some(Value::String(s)) => assert_eq!(s, "dodeca"),
                    other => panic!("Expected name to be 'dodeca', got {:?}", other),
                }
                match project.get("version") {
                    Some(Value::String(s)) => assert_eq!(s, "0.1.0"),
                    other => panic!("Expected version to be '0.1.0', got {:?}", other),
                }
            } else {
                panic!("Expected project to be a dict");
            }
        } else {
            panic!("Expected dict");
        }
    }

    #[test]
    fn test_parse_json() {
        let content = r#"{"name": "test", "count": 42}"#;
        let value = parse_json(content).unwrap();
        if let Value::Dict(map) = value {
            match map.get("name") {
                Some(Value::String(s)) => assert_eq!(s, "test"),
                other => panic!("Expected name to be 'test', got {:?}", other),
            }
            match map.get("count") {
                Some(Value::Int(n)) => assert_eq!(*n, 42),
                other => panic!("Expected count to be 42, got {:?}", other),
            }
        } else {
            panic!("Expected dict");
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
        let value = parse_yaml(content).unwrap();
        if let Value::Dict(map) = value {
            match map.get("name") {
                Some(Value::String(s)) => assert_eq!(s, "test"),
                other => panic!("Expected name to be 'test', got {:?}", other),
            }
            if let Some(Value::List(items)) = map.get("items") {
                assert_eq!(items.len(), 2);
                match &items[0] {
                    Value::String(s) => assert_eq!(s, "one"),
                    other => panic!("Expected first item to be 'one', got {:?}", other),
                }
            } else {
                panic!("Expected items to be a list");
            }
        } else {
            panic!("Expected dict");
        }
    }
}
