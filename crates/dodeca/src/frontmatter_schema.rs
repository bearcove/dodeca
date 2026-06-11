use std::collections::{HashMap, HashSet};

use dodeca_config::PageTypeSchema;
use facet_styx::{Meta, Schema, SchemaFile, Validator};
use facet_value::{DestructuredRef, Value};
use styx_tree::{
    Entry as StyxEntry, Object as StyxObject, Payload as StyxPayload, Sequence as StyxSequence,
    Value as StyxValue,
};

use crate::db::ParsedData;

#[derive(Debug, Clone)]
pub struct FrontmatterSchemaError {
    pub source_path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct LinkTarget {
    route: String,
    page_type: Option<String>,
}

pub fn validate(parsed: &[ParsedData]) -> Vec<FrontmatterSchemaError> {
    let Some(page_types) = crate::config::global_config().and_then(|config| {
        config
            .page_types
            .as_ref()
            .filter(|page_types| !page_types.is_empty())
    }) else {
        return Vec::new();
    };

    let schema_file = schema_file(page_types);
    let validator = Validator::new(&schema_file);
    let link_index = LinkIndex::build(parsed);
    let mut errors = Vec::new();

    for data in parsed {
        let Some(page_type) = frontmatter_type(&data.extra) else {
            continue;
        };

        let Some(schema) = page_types.get(page_type) else {
            errors.push(FrontmatterSchemaError {
                source_path: data.source_path.to_string(),
                message: format!("unknown frontmatter type '{page_type}'"),
            });
            continue;
        };

        let mut styx_value = facet_value_to_styx(&data.extra);
        let Some(styx_schema) = schema_file.schema.get(&Some(page_type.to_string())) else {
            errors.push(FrontmatterSchemaError {
                source_path: data.source_path.to_string(),
                message: format!("missing frontmatter schema for type '{page_type}'"),
            });
            continue;
        };
        coerce_enum_scalars(&mut styx_value, styx_schema);

        let result = validator.validate_value(&styx_value, styx_schema, "");
        for error in result.errors {
            errors.push(FrontmatterSchemaError {
                source_path: data.source_path.to_string(),
                message: format!("frontmatter schema '{page_type}': {error}"),
            });
        }

        validate_links(
            schema,
            &data.extra,
            "",
            page_types,
            &link_index,
            &mut HashSet::new(),
            &mut errors,
            data.source_path.as_str(),
        );
    }

    errors
}

fn schema_file(page_types: &HashMap<String, PageTypeSchema>) -> SchemaFile {
    SchemaFile {
        meta: Meta {
            id: "dodeca:frontmatter".to_string(),
            version: None,
            cli: Some("ddc".to_string()),
            description: Some("Dodeca frontmatter schemas".to_string()),
            lsp: None,
        },
        imports: None,
        schema: page_types
            .iter()
            .map(|(name, schema)| (Some(name.clone()), schema.to_styx_schema()))
            .collect(),
    }
}

fn frontmatter_type(extra: &Value) -> Option<&str> {
    extra
        .as_object()?
        .get("type")?
        .as_string()
        .map(|s| s.as_str())
}

#[derive(Debug, Clone, Default)]
struct LinkIndex {
    by_slug: HashMap<String, Vec<LinkTarget>>,
}

impl LinkIndex {
    fn build(parsed: &[ParsedData]) -> Self {
        let mut by_slug: HashMap<String, Vec<LinkTarget>> = HashMap::new();

        for data in parsed {
            let target = LinkTarget {
                route: data.route.to_string(),
                page_type: frontmatter_type(&data.extra).map(str::to_string),
            };

            for key in slug_keys(data) {
                by_slug.entry(key).or_default().push(target.clone());
            }
        }

        Self { by_slug }
    }

    fn get(&self, raw_target: &str) -> Option<&[LinkTarget]> {
        let key = normalize_slug(raw_target);
        self.by_slug.get(&key).map(Vec::as_slice)
    }
}

fn slug_keys(data: &ParsedData) -> Vec<String> {
    let mut keys = HashSet::new();

    let route_key = data.route.as_str().trim_matches('/');
    if !route_key.is_empty() {
        keys.insert(route_key.to_string());
        if let Some(leaf) = route_key.rsplit('/').next().filter(|leaf| !leaf.is_empty()) {
            keys.insert(leaf.to_string());
        }
    }

    let mut source_key = data
        .source_path
        .as_str()
        .trim_end_matches(".md")
        .to_string();
    if source_key == "_index" {
        source_key.clear();
    } else if let Some(section_key) = source_key.strip_suffix("/_index") {
        source_key = section_key.to_string();
    }
    if !source_key.is_empty() {
        keys.insert(source_key.clone());
        if let Some(leaf) = source_key
            .rsplit('/')
            .next()
            .filter(|leaf| !leaf.is_empty())
        {
            keys.insert(leaf.to_string());
        }
    }

    keys.into_iter().map(|key| normalize_slug(&key)).collect()
}

fn normalize_slug(slug: &str) -> String {
    let mut slug = slug.trim().trim_start_matches('/').trim_end_matches('/');
    if let Some(stripped) = slug.strip_suffix(".md") {
        slug = stripped;
    }
    if let Some(stripped) = slug.strip_suffix("/_index") {
        slug = stripped;
    }
    slug.to_string()
}

#[allow(clippy::too_many_arguments)]
fn validate_links(
    schema: &PageTypeSchema,
    value: &Value,
    path: &str,
    page_types: &HashMap<String, PageTypeSchema>,
    link_index: &LinkIndex,
    seen_types: &mut HashSet<String>,
    errors: &mut Vec<FrontmatterSchemaError>,
    source_path: &str,
) {
    if let Some(target_type) = schema.link_target_type() {
        validate_link_value(value, path, target_type, link_index, errors, source_path);
        return;
    }

    match schema {
        PageTypeSchema::Object(schema) => {
            let Some(object) = value.as_object() else {
                return;
            };
            let additional_schema = schema
                .0
                .iter()
                .find_map(|(key, value)| key.value.tag.is_some().then_some(value));

            for (field_name, field_value) in object.iter() {
                let name = field_name.as_str();
                let field_schema = schema
                    .0
                    .iter()
                    .find_map(|(key, schema)| (key.value.name() == Some(name)).then_some(schema))
                    .or(additional_schema);

                if let Some(field_schema) = field_schema {
                    let field_path = join_path(path, name);
                    validate_links(
                        field_schema,
                        field_value,
                        &field_path,
                        page_types,
                        link_index,
                        seen_types,
                        errors,
                        source_path,
                    );
                }
            }
        }
        PageTypeSchema::Seq(schema) => {
            let Some(array) = value.as_array() else {
                return;
            };
            for (index, item) in array.into_iter().enumerate() {
                validate_links(
                    &schema.0.0.value,
                    item,
                    &format!("{path}[{index}]"),
                    page_types,
                    link_index,
                    seen_types,
                    errors,
                    source_path,
                );
            }
        }
        PageTypeSchema::Tuple(schema) => {
            let Some(array) = value.as_array() else {
                return;
            };
            for (index, (item, item_schema)) in array.into_iter().zip(schema.0.iter()).enumerate() {
                validate_links(
                    &item_schema.value,
                    item,
                    &format!("{path}[{index}]"),
                    page_types,
                    link_index,
                    seen_types,
                    errors,
                    source_path,
                );
            }
        }
        PageTypeSchema::Map(schema) => {
            let Some(object) = value.as_object() else {
                return;
            };
            let (key_schema, value_schema) = match schema.0.len() {
                1 => (None, &schema.0[0].value),
                2 => (Some(&schema.0[0].value), &schema.0[1].value),
                _ => return,
            };

            for (key, item) in object.iter() {
                if let Some(key_schema) = key_schema {
                    validate_link_key(
                        key.as_str(),
                        path,
                        key_schema,
                        link_index,
                        errors,
                        source_path,
                    );
                }

                let field_path = join_path(path, key.as_str());
                validate_links(
                    value_schema,
                    item,
                    &field_path,
                    page_types,
                    link_index,
                    seen_types,
                    errors,
                    source_path,
                );
            }
        }
        PageTypeSchema::Union(schema) => {
            for variant in &schema.0 {
                validate_links(
                    &variant.value,
                    value,
                    path,
                    page_types,
                    link_index,
                    seen_types,
                    errors,
                    source_path,
                );
            }
        }
        PageTypeSchema::Optional(schema) => validate_links(
            &schema.0.0.value,
            value,
            path,
            page_types,
            link_index,
            seen_types,
            errors,
            source_path,
        ),
        PageTypeSchema::Flatten(schema) => validate_links(
            &schema.0.0.value,
            value,
            path,
            page_types,
            link_index,
            seen_types,
            errors,
            source_path,
        ),
        PageTypeSchema::Default(schema) => validate_links(
            &schema.0.1.value,
            value,
            path,
            page_types,
            link_index,
            seen_types,
            errors,
            source_path,
        ),
        PageTypeSchema::Deprecated(schema) => validate_links(
            &schema.0.1.value,
            value,
            path,
            page_types,
            link_index,
            seen_types,
            errors,
            source_path,
        ),
        PageTypeSchema::Type { name: Some(name) } if seen_types.insert(name.clone()) => {
            if let Some(schema) = page_types.get(name) {
                validate_links(
                    schema,
                    value,
                    path,
                    page_types,
                    link_index,
                    seen_types,
                    errors,
                    source_path,
                );
            }
            seen_types.remove(name);
        }
        _ => {}
    }
}

fn validate_link_key(
    key: &str,
    path: &str,
    schema: &PageTypeSchema,
    link_index: &LinkIndex,
    errors: &mut Vec<FrontmatterSchemaError>,
    source_path: &str,
) {
    if let Some(target_type) = schema.link_target_type() {
        validate_link(key, path, target_type, link_index, errors, source_path);
    }
}

fn validate_link_value(
    value: &Value,
    path: &str,
    target_type: &str,
    link_index: &LinkIndex,
    errors: &mut Vec<FrontmatterSchemaError>,
    source_path: &str,
) {
    let Some(target) = value.as_string() else {
        return;
    };
    validate_link(
        target.as_str(),
        path,
        target_type,
        link_index,
        errors,
        source_path,
    );
}

fn validate_link(
    raw_target: &str,
    path: &str,
    target_type: &str,
    link_index: &LinkIndex,
    errors: &mut Vec<FrontmatterSchemaError>,
    source_path: &str,
) {
    let Some(targets) = link_index.get(raw_target) else {
        errors.push(FrontmatterSchemaError {
            source_path: source_path.to_string(),
            message: format!(
                "frontmatter link {} target '{}' not found for type {}",
                display_path(path),
                raw_target,
                target_type
            ),
        });
        return;
    };

    if targets
        .iter()
        .any(|target| target.page_type.as_deref() == Some(target_type))
    {
        return;
    }

    let mut found: Vec<String> = targets
        .iter()
        .map(|target| {
            format!(
                "{} ({})",
                target.route,
                target.page_type.as_deref().unwrap_or("untyped")
            )
        })
        .collect();
    found.sort();
    found.dedup();

    errors.push(FrontmatterSchemaError {
        source_path: source_path.to_string(),
        message: format!(
            "frontmatter link {} target '{}' has wrong type; expected {}, found {}",
            display_path(path),
            raw_target,
            target_type,
            found.join(", ")
        ),
    });
}

fn join_path(parent: &str, field: &str) -> String {
    if parent.is_empty() {
        field.to_string()
    } else {
        format!("{parent}.{field}")
    }
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "<root>".to_string()
    } else {
        path.to_string()
    }
}

/// Lift untagged scalars to tagged variants where the schema slot is an enum
/// whose matching variant is unit-shaped. TOML/YAML can't produce styx tags;
/// this pass bridges format-poor frontmatter to the unweakened styx schema.
fn coerce_enum_scalars(value: &mut StyxValue, schema: &Schema) {
    match schema {
        Schema::Enum(enum_schema) => {
            if value.tag.is_some() {
                return;
            }
            let Some(text) = value.scalar_text() else {
                return;
            };
            let text = text.to_string();
            let all_unit_variants = enum_schema
                .0
                .iter()
                .all(|(_, schema)| schema_is_unit_variant(schema));
            for (variant_name, variant_schema) in &enum_schema.0 {
                if variant_name.value != text {
                    continue;
                }
                if schema_is_unit_variant(variant_schema) {
                    *value = StyxValue::tag(variant_name.value.clone());
                }
                return;
            }
            if all_unit_variants {
                *value = StyxValue::tag(text);
            }
        }
        Schema::Optional(opt) => coerce_enum_scalars(value, &opt.0.0.value),
        Schema::Default(d) => coerce_enum_scalars(value, &d.0.1.value),
        Schema::Deprecated(d) => coerce_enum_scalars(value, &d.0.1.value),
        Schema::Object(obj_schema) => {
            let Some(StyxPayload::Object(object)) = value.payload.as_mut() else {
                return;
            };
            let additional_schema = obj_schema
                .0
                .iter()
                .find_map(|(key, value)| key.value.tag.is_some().then_some(value));
            for entry in &mut object.entries {
                let Some(field_name) = entry.key.scalar_text() else {
                    continue;
                };
                let field_name = field_name.to_string();
                let field_schema = obj_schema
                    .0
                    .iter()
                    .find_map(|(key, schema)| {
                        (key.value.name() == Some(field_name.as_str())).then_some(schema)
                    })
                    .or(additional_schema);
                if let Some(field_schema) = field_schema {
                    coerce_enum_scalars(&mut entry.value, field_schema);
                }
            }
        }
        Schema::Seq(seq) => {
            let Some(StyxPayload::Sequence(seq_payload)) = value.payload.as_mut() else {
                return;
            };
            let item_schema = &seq.0.0.value;
            for item in &mut seq_payload.items {
                coerce_enum_scalars(item, item_schema);
            }
        }
        Schema::Tuple(tuple) => {
            let Some(StyxPayload::Sequence(seq_payload)) = value.payload.as_mut() else {
                return;
            };
            for (item, item_schema) in seq_payload.items.iter_mut().zip(tuple.0.iter()) {
                coerce_enum_scalars(item, &item_schema.value);
            }
        }
        _ => {}
    }
}

fn schema_is_unit_variant(schema: &Schema) -> bool {
    match schema {
        Schema::Unit => true,
        Schema::Type { name: None } => true,
        Schema::Type { name: Some(n) } if n == "unit" => true,
        _ => false,
    }
}

fn facet_value_to_styx(value: &Value) -> StyxValue {
    match value.destructure_ref() {
        DestructuredRef::Null => StyxValue::unit(),
        DestructuredRef::Bool(value) => StyxValue::scalar(value.to_string()),
        DestructuredRef::Number(value) => {
            if let Some(value) = value.to_i64() {
                StyxValue::scalar(value.to_string())
            } else if let Some(value) = value.to_u64() {
                StyxValue::scalar(value.to_string())
            } else if let Some(value) = value.to_f64() {
                StyxValue::scalar(value.to_string())
            } else {
                StyxValue::scalar(value.to_f64_lossy().to_string())
            }
        }
        DestructuredRef::String(value) => StyxValue::scalar(value.as_str()),
        DestructuredRef::Bytes(value) => StyxValue::scalar(format!("{:?}", value)),
        DestructuredRef::Array(array) => StyxValue {
            tag: None,
            payload: Some(StyxPayload::Sequence(StyxSequence {
                items: array.into_iter().map(facet_value_to_styx).collect(),
                span: None,
            })),
            span: None,
        },
        DestructuredRef::Object(object) => StyxValue {
            tag: None,
            payload: Some(StyxPayload::Object(StyxObject {
                entries: object
                    .iter()
                    .map(|(key, value)| StyxEntry {
                        key: StyxValue::scalar(key.as_str()),
                        value: facet_value_to_styx(value),
                        doc_comment: None,
                    })
                    .collect(),
                span: None,
            })),
            span: None,
        },
        DestructuredRef::DateTime(value) => StyxValue::scalar(format!("{value:?}")),
        DestructuredRef::QName(value) => StyxValue::scalar(format!("{value:?}")),
        DestructuredRef::Uuid(value) => StyxValue::scalar(format!("{value:?}")),
        DestructuredRef::Char(value) => StyxValue::scalar(value.to_string()),
        other => StyxValue::scalar(format!("{other:?}")),
    }
}
