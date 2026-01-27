//! Dodeca data cell (cell-data)
//!
//! This cell handles loading and parsing data files (JSON, TOML, YAML).

use cell_data_proto::{DataFormat, DataLoader, DataLoaderDispatcher, LoadDataResult, Value};
use dodeca_cell_runtime::run_cell;
use facet_format::{DeserializeError, FormatDeserializer, FormatParser};

/// Data loader implementation
#[derive(Clone)]
pub struct DataLoaderImpl;

impl DataLoader for DataLoaderImpl {
    async fn load_data(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        content: String,
        format: DataFormat,
    ) -> LoadDataResult {
        match parse_data(&content, format) {
            Ok(value) => LoadDataResult::Success { value },
            Err(e) => LoadDataResult::Error { message: e },
        }
    }
}

/// Parse data using dyn dispatch for reduced monomorphization
fn parse_data(content: &str, format: DataFormat) -> Result<Value, String> {
    match format {
        DataFormat::Json => {
            let mut parser = facet_json::JsonParser::<true>::new(content.as_bytes());
            deserialize_value(&mut parser).map_err(|e| format!("JSON parse error: {e}"))
        }
        DataFormat::Toml => {
            let mut parser = facet_toml::TomlParser::new(content)
                .map_err(|e| format!("TOML parse error: {e}"))?;
            deserialize_value(&mut parser).map_err(|e| format!("TOML parse error: {e}"))
        }
        DataFormat::Yaml => {
            let mut parser = facet_yaml::YamlParser::new(content);
            deserialize_value(&mut parser).map_err(|e| format!("YAML parse error: {e}"))
        }
    }
}

/// Deserialize a Value using dynamic dispatch.
///
/// This function only has one monomorphization regardless of parser type.
fn deserialize_value(parser: &mut dyn FormatParser<'_>) -> Result<Value, DeserializeError> {
    let mut de = FormatDeserializer::new(parser);
    de.deserialize()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("data", |_handle| DataLoaderDispatcher::new(DataLoaderImpl))
}
