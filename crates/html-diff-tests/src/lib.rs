//! HTML diff path translation
//!
//! Translates facet-diff EditOps (from GumTree/Chawathe) into DOM Patches.

#[macro_use]
mod macros;

pub mod apply;

pub use dodeca_protocol::{NodePath, Patch};
use facet_diff::{EditOp, PathSegment, tree_diff};
use facet_html::{self as html};
use facet_html_dom::*;

/// Diff two HTML documents and return DOM patches.
pub fn diff_html(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    let old_doc: Html =
        facet_html::from_str(old_html).map_err(|e| format!("Failed to parse old HTML: {e}"))?;
    let new_doc: Html =
        facet_html::from_str(new_html).map_err(|e| format!("Failed to parse new HTML: {e}"))?;

    let edit_ops = tree_diff(&old_doc, &new_doc);
    Ok(translate_to_patches(&edit_ops, &new_doc))
}

/// Diff with debug tracing of raw edit ops.
#[cfg(feature = "tracing")]
pub fn diff_html_debug(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    let old_doc: Html =
        facet_html::from_str(old_html).map_err(|e| format!("Failed to parse old HTML: {e}"))?;
    let new_doc: Html =
        facet_html::from_str(new_html).map_err(|e| format!("Failed to parse new HTML: {e}"))?;

    let edit_ops = tree_diff(&old_doc, &new_doc);

    tracing::info!(count = edit_ops.len(), "Edit ops from facet-diff");
    for op in &edit_ops {
        tracing::info!(?op, "edit op");
    }

    let patches = translate_to_patches(&edit_ops, &new_doc);

    tracing::info!(count = patches.len(), "Translated patches");
    for patch in &patches {
        tracing::info!(?patch, "patch");
    }

    Ok(patches)
}

/// Diff with debug tracing of raw edit ops.
#[cfg(not(feature = "tracing"))]
pub fn diff_html_debug(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    diff_html(old_html, new_html)
}

/// Translate facet-diff EditOps into DOM Patches.
///
/// Simple approach: translate each op directly. Update ops for non-leaf paths
/// (elements, containers) are skipped as they just bubble up from leaf changes.
pub fn translate_to_patches(edit_ops: &[EditOp], new_doc: &Html) -> Vec<Patch> {
    let mut patches = Vec::new();

    for op in edit_ops {
        if let Some(patch) = translate_op(op, new_doc) {
            patches.push(patch);
        }
    }

    deduplicate_patches(patches)
}

/// Remove redundant patches.
fn deduplicate_patches(patches: Vec<Patch>) -> Vec<Patch> {
    use std::collections::HashSet;

    // Collect paths where we're replacing inner HTML
    let replace_inner_paths: HashSet<Vec<usize>> = patches
        .iter()
        .filter_map(|p| match p {
            Patch::ReplaceInnerHtml { path, .. } => Some(path.0.clone()),
            _ => None,
        })
        .collect();

    // Collect SetAttribute paths to filter RemoveAttribute
    let set_attrs: HashSet<(Vec<usize>, String)> = patches
        .iter()
        .filter_map(|p| match p {
            Patch::SetAttribute { path, name, .. } => Some((path.0.clone(), name.clone())),
            _ => None,
        })
        .collect();

    patches
        .into_iter()
        .filter(|p| {
            // If we have SetAttribute, remove corresponding RemoveAttribute
            if let Patch::RemoveAttribute { path, name } = p {
                if set_attrs.contains(&(path.0.clone(), name.clone())) {
                    return false;
                }
            }

            // If we're replacing inner HTML at path X, skip child operations
            let child_path = match p {
                Patch::InsertBefore { path, .. }
                | Patch::AppendChild { path, .. }
                | Patch::Remove { path }
                | Patch::Move { to: path, .. } => Some(&path.0),
                _ => None,
            };

            if let Some(child) = child_path {
                for parent in &replace_inner_paths {
                    // Check if child is a descendant of parent (starts with parent's path)
                    if child.len() > parent.len() && child.starts_with(parent) {
                        return false;
                    }
                    // Also skip if it's a direct child (same length + 1)
                    if child.len() == parent.len() + 1 && child[..parent.len()] == parent[..] {
                        return false;
                    }
                    // Skip if it's at the same level but parent path is empty (body level)
                    if parent.is_empty() && !child.is_empty() {
                        return false;
                    }
                }
            }

            true
        })
        .collect()
}

/// Translate a single EditOp to a DOM Patch.
fn translate_op(op: &EditOp, new_doc: &Html) -> Option<Patch> {
    debug!("translate_op: op={op:?}");
    match op {
        EditOp::Insert { path, .. } => translate_insert(&path.0, new_doc),
        EditOp::Delete { path, .. } => translate_delete(&path.0),
        EditOp::Update { path, .. } => translate_update(&path.0, new_doc),
        EditOp::Move {
            old_path,
            new_path,
            ..
        } => {
            // Only translate element moves, not internal structure moves (attrs, text, etc.)
            if path_target(&old_path.0) != PathTarget::Element {
                return None;
            }
            let from = to_dom_path(&old_path.0);
            let to = to_dom_path(&new_path.0);
            Some(Patch::Move {
                from: NodePath(from),
                to: NodePath(to),
            })
        }
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Convert a facet path to a DOM path.
///
/// Facet path: [Field("body"), Field("children"), Index(1), Variant("Div"), Index(0), Field("children"), Index(2)]
/// DOM path: [1, 2] (just the indices from children arrays)
fn to_dom_path(segments: &[PathSegment]) -> Vec<usize> {
    let mut dom_path = Vec::new();
    let mut i = 0;

    while i < segments.len() {
        // Look for Field("children") followed by Index(n)
        if let PathSegment::Field(name) = &segments[i] {
            if name == "children" || name == "li" {
                if let Some(PathSegment::Index(idx)) = segments.get(i + 1) {
                    dom_path.push(*idx);
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }

    dom_path
}

/// What does this path point to?
#[derive(Debug, Clone, PartialEq)]
enum PathTarget {
    /// Text node content
    Text,
    /// An attribute (with name)
    Attribute(String),
    /// An element node
    Element,
    /// Something else (children array, etc.)
    Other,
}

/// Analyze what a path targets.
fn path_target(segments: &[PathSegment]) -> PathTarget {
    let last = segments.last();
    let second_last = segments.len().checked_sub(2).and_then(|i| segments.get(i));

    // Check for text: ends with Variant("Text") or Variant("_text")
    if let Some(PathSegment::Variant(name)) = last {
        if name == "Text" || name == "_text" {
            return PathTarget::Text;
        }
    }
    // Also check Index(0) after Variant("Text")
    if let (Some(PathSegment::Variant(name)), Some(PathSegment::Index(0))) = (second_last, last) {
        if name == "Text" || name == "_text" {
            return PathTarget::Text;
        }
    }

    // Check for attribute: Field("attrs") followed by Field(attr_name)
    if let (Some(PathSegment::Field(parent)), Some(PathSegment::Field(attr))) = (second_last, last)
    {
        if parent == "attrs" {
            return PathTarget::Attribute(attr.to_string());
        }
    }

    // Check for direct attributes (href, src, etc. flattened from attrs)
    if let Some(PathSegment::Field(name)) = last {
        if is_direct_attribute(name) {
            return PathTarget::Attribute(name.to_string());
        }
        // If it ends with "children" or other structural fields, it's Other
        if name == "children" || name == "attrs" || name == "body" || name == "head" {
            return PathTarget::Other;
        }
    }

    // Check for element: ends with Variant(ElementName) followed by Index(0)
    if let (Some(PathSegment::Variant(name)), Some(PathSegment::Index(0))) = (second_last, last) {
        if name != "Text" && name != "_text" {
            return PathTarget::Element;
        }
    }

    // If it ends with Index after children, it's an element in a children array
    if let Some(PathSegment::Index(_)) = last {
        if let Some(PathSegment::Field(name)) = second_last {
            if name == "children" || name == "li" {
                return PathTarget::Element;
            }
        }
    }

    PathTarget::Other
}

fn is_direct_attribute(name: &str) -> bool {
    matches!(
        name,
        "href" | "src" | "alt" | "target" | "rel" | "download" | "type" | "action" | "method"
            | "name" | "value" | "placeholder" | "class" | "id" | "style"
    )
}

/// Translate an Insert operation.
fn translate_insert(segments: &[PathSegment], new_doc: &Html) -> Option<Patch> {
    let target = path_target(segments);
    let dom_path = to_dom_path(segments);

    debug!("translate_insert: segments={segments:?}, dom_path={dom_path:?}, target={target:?}");

    match target {
        PathTarget::Element => {
            // Navigate to the node and serialize it
            let peek = facet::Peek::new(new_doc);
            let node_peek = navigate_peek(peek, segments)?;
            let html = serialize_to_html(node_peek)?;

            if dom_path.is_empty() {
                // Inserting at body level - append to body
                Some(Patch::AppendChild {
                    path: NodePath(vec![]),
                    html,
                })
            } else {
                // Insert before the node at this path
                Some(Patch::InsertBefore {
                    path: NodePath(dom_path),
                    html,
                })
            }
        }
        PathTarget::Attribute(name) => {
            // Get the attribute value
            let value = get_attribute_value(new_doc, segments, &name)?;
            // DOM path is the element, not the attribute
            let elem_path = to_dom_path(&segments[..segments.len().saturating_sub(2)]);
            Some(Patch::SetAttribute {
                path: NodePath(elem_path),
                name,
                value,
            })
        }
        PathTarget::Text => {
            // Get the text content
            let text = get_text_value(new_doc, segments)?;
            // DOM path is the parent element
            let parent_path = if dom_path.is_empty() {
                vec![]
            } else {
                dom_path[..dom_path.len() - 1].to_vec()
            };
            Some(Patch::SetText {
                path: NodePath(parent_path),
                text,
            })
        }
        PathTarget::Other => {
            // Check if this is an Insert at a "children" field - replace inner HTML of parent
            if let Some(PathSegment::Field(name)) = segments.last() {
                if name == "children" {
                    debug!("handling children insert");
                    // Get the parent element's children and serialize them
                    let parent_segments = &segments[..segments.len() - 1];
                    debug!("parent_segments={parent_segments:?}");
                    let peek = facet::Peek::new(new_doc);
                    if let Some(parent_peek) = navigate_peek(peek, parent_segments) {
                        debug!("navigated to parent, shape={:?}", parent_peek.shape());
                        // Handle Option<Body> or similar by unwrapping
                        let struct_peek = if let Ok(opt) = parent_peek.into_option() {
                            opt.value()
                        } else {
                            let peek2 = facet::Peek::new(new_doc);
                            navigate_peek(peek2, parent_segments)
                        };
                        if let Some(struct_peek) = struct_peek {
                            if let Ok(s) = struct_peek.into_struct() {
                                debug!("parent is struct");
                                if let Ok(children) = s.field_by_name("children") {
                                    debug!("got children field, shape={:?}", children.shape());
                                    if let Ok(list) = children.into_list() {
                                        debug!("children is list with {} items", list.len());
                                        let mut children_html = String::new();
                                        for child in list.iter() {
                                            if let Some(html) = serialize_to_html(child) {
                                                children_html.push_str(&html);
                                            }
                                        }
                                        debug!("serialized children_html={children_html:?}");
                                        return Some(Patch::ReplaceInnerHtml {
                                            path: NodePath(dom_path),
                                            html: children_html,
                                        });
                                    } else {
                                        debug!("children is NOT a list");
                                    }
                                } else {
                                    debug!("no children field in struct");
                                }
                            } else {
                                debug!("unwrapped parent is NOT a struct");
                            }
                        } else {
                            debug!("parent Option is None or couldn't unwrap");
                        }
                    } else {
                        debug!("failed to navigate to parent");
                    }
                }
            }
            None
        }
    }
}

/// Translate a Delete operation.
fn translate_delete(segments: &[PathSegment]) -> Option<Patch> {
    let target = path_target(segments);
    let dom_path = to_dom_path(segments);

    debug!("translate_delete: segments={segments:?}");
    debug!("  dom_path={dom_path:?}, target={target:?}");

    match target {
        PathTarget::Element => {
            if dom_path.is_empty() {
                None // Can't delete body
            } else {
                Some(Patch::Remove {
                    path: NodePath(dom_path),
                })
            }
        }
        PathTarget::Attribute(name) => {
            let elem_path = to_dom_path(&segments[..segments.len().saturating_sub(2)]);
            Some(Patch::RemoveAttribute {
                path: NodePath(elem_path),
                name,
            })
        }
        PathTarget::Text | PathTarget::Other => {
            // Text deletion or structural - handled by parent operations
            None
        }
    }
}

/// Translate an Update operation.
fn translate_update(segments: &[PathSegment], new_doc: &Html) -> Option<Patch> {
    let target = path_target(segments);
    let dom_path = to_dom_path(segments);

    debug!("translate_update: segments={segments:?}");
    debug!("  dom_path={dom_path:?}, target={target:?}");

    match target {
        PathTarget::Text => {
            let text = get_text_value(new_doc, segments)?;
            let parent_path = if dom_path.is_empty() {
                vec![]
            } else {
                dom_path[..dom_path.len() - 1].to_vec()
            };
            Some(Patch::SetText {
                path: NodePath(parent_path),
                text,
            })
        }
        PathTarget::Attribute(name) => {
            let value = get_attribute_value(new_doc, segments, &name)?;
            let elem_path = to_dom_path(&segments[..segments.len().saturating_sub(2)]);
            Some(Patch::SetAttribute {
                path: NodePath(elem_path),
                name,
                value,
            })
        }
        // Element/Other updates are NOT translated - they just bubble up from leaf changes
        PathTarget::Element | PathTarget::Other => None,
    }
}

/// Navigate a Peek value following path segments.
fn navigate_peek<'mem, 'facet>(
    mut peek: facet::Peek<'mem, 'facet>,
    segments: &[PathSegment],
) -> Option<facet::Peek<'mem, 'facet>> {
    for segment in segments {
        peek = match segment {
            PathSegment::Field(name) => {
                if let Ok(s) = peek.into_struct() {
                    s.field_by_name(name).ok()?
                } else if let Ok(opt) = peek.into_option() {
                    let inner = opt.value()?;
                    if let Ok(s) = inner.into_struct() {
                        s.field_by_name(name).ok()?
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            PathSegment::Index(idx) => {
                if let Ok(list) = peek.into_list() {
                    list.get(*idx)?
                } else if let Ok(opt) = peek.into_option() {
                    if *idx == 0 {
                        opt.value()?
                    } else {
                        return None;
                    }
                } else if let Ok(e) = peek.into_enum() {
                    e.field(*idx).ok()??
                } else {
                    return None;
                }
            }
            PathSegment::Variant(_) => {
                // Enum variant - value already IS that variant, continue
                peek
            }
            PathSegment::Key(key) => {
                if let Ok(map) = peek.into_map() {
                    for (k, v) in map.iter() {
                        if let Some(s) = k.as_str() {
                            if s == key {
                                return Some(v);
                            }
                        }
                    }
                    return None;
                } else {
                    return None;
                }
            }
        };
    }
    Some(peek)
}

/// Serialize a Peek value to HTML.
fn serialize_to_html(peek: facet::Peek<'_, '_>) -> Option<String> {
    let mut serializer = html::HtmlSerializer::new();
    facet_dom::serialize(&mut serializer, peek).ok()?;
    let bytes = serializer.finish();
    String::from_utf8(bytes).ok()
}

/// Get an attribute value by navigating to it.
fn get_attribute_value(doc: &Html, segments: &[PathSegment], attr_name: &str) -> Option<String> {
    let peek = facet::Peek::new(doc);

    // Navigate to the element (skip the last 2 segments which are attrs/attr_name or just attr_name)
    let elem_segments = if segments.len() >= 2 {
        if let Some(PathSegment::Field(f)) = segments.get(segments.len() - 2) {
            if f == "attrs" {
                &segments[..segments.len() - 2]
            } else {
                &segments[..segments.len() - 1]
            }
        } else {
            &segments[..segments.len() - 1]
        }
    } else {
        return None;
    };

    let elem_peek = navigate_peek(peek, elem_segments)?;

    // Try to get the attrs struct
    if let Ok(s) = elem_peek.into_struct() {
        // Try attrs.{attr_name}
        if let Ok(attrs) = s.field_by_name("attrs") {
            if let Ok(attrs_struct) = attrs.into_struct() {
                // Map common attr names
                let field_name = match attr_name {
                    "class" => "class",
                    "id" => "id",
                    "style" => "style",
                    "title" => "tooltip",
                    other => other,
                };
                if let Ok(val) = attrs_struct.field_by_name(field_name) {
                    if let Ok(opt) = val.into_option() {
                        if let Some(inner) = opt.value() {
                            if let Some(s) = inner.as_str() {
                                return Some(s.to_string());
                            }
                        }
                    } else if let Some(s) = val.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        // Try direct attribute field
        if let Ok(val) = s.field_by_name(attr_name) {
            if let Ok(opt) = val.into_option() {
                if let Some(inner) = opt.value() {
                    if let Some(s) = inner.as_str() {
                        return Some(s.to_string());
                    }
                }
            } else if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}

/// Get text content by navigating to it.
fn get_text_value(doc: &Html, segments: &[PathSegment]) -> Option<String> {
    let peek = facet::Peek::new(doc);
    let text_peek = navigate_peek(peek, segments)?;

    // Text nodes are just String values
    if let Some(s) = text_peek.as_str() {
        return Some(s.to_string());
    }

    // Or might be wrapped in enum - try to get field 0
    let peek2 = facet::Peek::new(doc);
    let text_peek2 = navigate_peek(peek2, segments)?;
    if let Ok(e) = text_peek2.into_enum() {
        if let Ok(Some(field)) = e.field(0) {
            if let Some(s) = field.as_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}
