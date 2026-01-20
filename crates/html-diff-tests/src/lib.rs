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

/// Diff with debug output of raw edit ops.
pub fn diff_html_debug(
    old_html: &str,
    new_html: &str,
    print_ops: bool,
) -> Result<Vec<Patch>, String> {
    let old_doc: Html =
        facet_html::from_str(old_html).map_err(|e| format!("Failed to parse old HTML: {e}"))?;
    let new_doc: Html =
        facet_html::from_str(new_html).map_err(|e| format!("Failed to parse new HTML: {e}"))?;

    let edit_ops = tree_diff(&old_doc, &new_doc);

    if print_ops {
        println!("=== Edit ops from facet-diff ({} total) ===", edit_ops.len());
        for op in &edit_ops {
            println!("  {op:?}");
        }
        println!("=== End edit ops ===");
    }

    Ok(translate_to_patches(&edit_ops, &new_doc))
}

/// Translate facet-diff EditOps into DOM Patches.
///
/// Strategy:
/// 1. Find Insert+Delete pairs at "children" paths -> ReplaceInnerHtml for parent
/// 2. Body-level replacement only if NO specific leaf operations exist
/// 3. Process remaining ops normally
pub fn translate_to_patches(edit_ops: &[EditOp], new_doc: &Html) -> Vec<Patch> {
    // Find all Insert+Delete pairs at paths ending in "children"
    // These need ReplaceInnerHtml for the parent element
    let mut children_replacements: Vec<Vec<PathSegment>> = Vec::new();
    for op in edit_ops {
        if let EditOp::Insert { path, .. } = op {
            if let Some(PathSegment::Field(name)) = path.0.last() {
                if name == "children" {
                    // Check if there's a corresponding Delete
                    let has_delete = edit_ops.iter().any(|op2| {
                        if let EditOp::Delete { path: del_path, .. } = op2 {
                            del_path.0 == path.0
                        } else {
                            false
                        }
                    });
                    if has_delete {
                        children_replacements.push(path.0.clone());
                    }
                }
            }
        }
    }

    // Body-level replacement paths
    let body_path = vec![PathSegment::Field("body".into())];
    let body_children_path = vec![
        PathSegment::Field("body".into()),
        PathSegment::Field("children".into()),
    ];

    // If we have children replacements at non-body level, handle those
    let non_body_children_replacements: Vec<_> = children_replacements
        .iter()
        .filter(|p| **p != body_children_path)
        .cloned()
        .collect();

    if !non_body_children_replacements.is_empty() {
        let mut patches = Vec::new();

        // Emit ReplaceInnerHtml for each children replacement
        for path in &non_body_children_replacements {
            // Path to parent element (without "children")
            let parent_path = &path[..path.len() - 1];
            let dom_path = to_dom_path(parent_path);

            // Get the new children HTML
            let peek = facet::Peek::new(new_doc);
            if let Some(parent_peek) = navigate_peek(peek, parent_path) {
                if let Ok(s) = parent_peek.into_struct() {
                    if let Ok(children) = s.field_by_name("children") {
                        if let Ok(list) = children.into_list() {
                            let mut children_html = String::new();
                            for child in list.iter() {
                                if let Some(html) = serialize_to_html(child) {
                                    children_html.push_str(&html);
                                }
                            }
                            patches.push(Patch::ReplaceInnerHtml {
                                path: NodePath(dom_path),
                                html: children_html,
                            });
                        }
                    }
                }
            }
        }

        // Process remaining ops that aren't dominated by the replacements
        for op in edit_ops {
            // Skip Insert/Delete on paths we're replacing
            let dominated = match op {
                EditOp::Insert { path, .. } | EditOp::Delete { path, .. } => {
                    non_body_children_replacements.iter().any(|rp| {
                        // Skip if path starts with a replacement path
                        path.0.starts_with(rp) || *rp == path.0
                    })
                }
                _ => false,
            };
            if !dominated {
                if let Some(patch) = translate_op(op, new_doc) {
                    patches.push(patch);
                }
            }
        }

        return deduplicate_attr_patches(patches);
    }

    // Body-level replacement: if body Insert+Delete exists, use ReplaceInnerHtml
    // This handles cases where the body structure changed significantly
    let body_insert_delete = edit_ops.iter().any(|op| {
        if let EditOp::Insert { path, .. } = op {
            path.0 == body_path || path.0 == body_children_path
        } else {
            false
        }
    }) && edit_ops.iter().any(|op| {
        if let EditOp::Delete { path, .. } = op {
            path.0 == body_path || path.0 == body_children_path
        } else {
            false
        }
    });

    if body_insert_delete {
        if let Some(body) = &new_doc.body {
            let mut children_html = String::new();
            for child in &body.children {
                let child_peek = facet::Peek::new(child);
                let mut serializer = html::HtmlSerializer::new();
                let _ = facet_dom::serialize(&mut serializer, child_peek);
                if let Ok(html) = String::from_utf8(serializer.finish()) {
                    children_html.push_str(&html);
                }
            }
            return vec![Patch::ReplaceInnerHtml {
                path: NodePath(vec![]),
                html: children_html,
            }];
        }
    }

    // Standard processing for all other cases
    // First, collect all element Insert paths so we can skip dominated operations
    let element_inserts: Vec<&[PathSegment]> = edit_ops
        .iter()
        .filter_map(|op| {
            if let EditOp::Insert { path, .. } = op {
                // Check if this is an element insert (ends with Index after children)
                if path_target(&path.0) == PathTarget::Element {
                    return Some(path.0.as_slice());
                }
            }
            None
        })
        .collect();

    let mut patches = Vec::new();
    for op in edit_ops {
        // Skip if this operation is dominated by an element insert
        let dominated = match op {
            EditOp::Insert { path, .. }
            | EditOp::Delete { path, .. }
            | EditOp::Update { path, .. } => {
                // Skip if this is a descendant of an element insert (or attribute/text within it)
                element_inserts.iter().any(|insert_path| {
                    // This path is dominated if it starts with insert_path and is deeper
                    path.0.len() > insert_path.len() && path.0.starts_with(insert_path)
                })
            }
            _ => false,
        };

        if !dominated {
            if let Some(patch) = translate_op(op, new_doc) {
                patches.push(patch);
            }
        }
    }
    deduplicate_attr_patches(patches)
}

/// Remove redundant attribute patches.
fn deduplicate_attr_patches(mut patches: Vec<Patch>) -> Vec<Patch> {
    use std::collections::HashSet;

    // If we have SetAttribute for (path, name), remove any RemoveAttribute for the same (path, name).
    // This happens because facet-diff emits Delete for old value and Insert/Update for new value.
    let set_attrs: HashSet<(Vec<usize>, String)> = patches
        .iter()
        .filter_map(|p| match p {
            Patch::SetAttribute { path, name, .. } => Some((path.0.clone(), name.clone())),
            _ => None,
        })
        .collect();

    patches.retain(|p| match p {
        Patch::RemoveAttribute { path, name } => !set_attrs.contains(&(path.0.clone(), name.clone())),
        _ => true,
    });

    patches
}

/// Translate a single EditOp to a DOM Patch.
fn translate_op(op: &EditOp, new_doc: &Html) -> Option<Patch> {
    match op {
        EditOp::Insert { path, .. } => translate_insert(&path.0, new_doc),
        EditOp::Delete { path, .. } => translate_delete(&path.0),
        EditOp::Update { path, .. } => translate_update(&path.0, new_doc),
        EditOp::Move { .. } => {
            // Move = Delete + Insert, facet-diff should emit those separately
            None
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

    debug!("translate_insert: segments={segments:?}");
    debug!("  dom_path={dom_path:?}, target={target:?}");

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
            // Check if this is an insert on a "children" field
            if let Some(PathSegment::Field(name)) = segments.last() {
                if name == "children" {
                    // Try to get text content first
                    if let Some(text) = try_get_element_text(new_doc, segments) {
                        return Some(Patch::SetText {
                            path: NodePath(dom_path),
                            text,
                        });
                    }
                    // Otherwise try to get element content (Replace the whole body contents)
                    if let Some(html) = try_get_element_html(new_doc, segments) {
                        return Some(Patch::Replace {
                            path: NodePath(dom_path.iter().chain(std::iter::once(&0)).copied().collect()),
                            html,
                        });
                    }
                }
            }
            None
        }
    }
}

/// Try to get text content from an element's children.
fn try_get_element_text(doc: &Html, segments: &[PathSegment]) -> Option<String> {
    // Navigate to the element (path without the final "children")
    let elem_segments = if segments.last() == Some(&PathSegment::Field("children".into())) {
        &segments[..segments.len() - 1]
    } else {
        return None;
    };

    let peek = facet::Peek::new(doc);
    let elem_peek = navigate_peek(peek, elem_segments)?;

    // The element might be wrapped in Option (e.g., body field)
    let elem_struct = if let Ok(s) = elem_peek.into_struct() {
        s
    } else {
        // Try unwrapping Option first
        let peek2 = facet::Peek::new(doc);
        let elem_peek2 = navigate_peek(peek2, elem_segments)?;
        if let Ok(opt) = elem_peek2.into_option() {
            opt.value()?.into_struct().ok()?
        } else {
            return None;
        }
    };

    // Try to get the children field and check for text content
    if let Ok(children) = elem_struct.field_by_name("children") {
        if let Ok(list) = children.into_list() {
            // Check if first child is text
            if let Some(first) = list.get(0) {
                // Text variants are either FlowContent::Text or PhrasingContent::Text
                if let Ok(e) = first.into_enum() {
                    if e.variant_name_active() == Ok("Text") {
                        if let Ok(Some(field)) = e.field(0) {
                            if let Some(s) = field.as_str() {
                                return Some(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Try to get HTML for an element's first child (for replacing content).
fn try_get_element_html(doc: &Html, segments: &[PathSegment]) -> Option<String> {
    // Navigate to the element (path without the final "children")
    let elem_segments = if segments.last() == Some(&PathSegment::Field("children".into())) {
        &segments[..segments.len() - 1]
    } else {
        return None;
    };

    let peek = facet::Peek::new(doc);
    let elem_peek = navigate_peek(peek, elem_segments)?;

    // The element might be wrapped in Option (e.g., body field)
    let elem_struct = if let Ok(s) = elem_peek.into_struct() {
        s
    } else {
        let peek2 = facet::Peek::new(doc);
        let elem_peek2 = navigate_peek(peek2, elem_segments)?;
        if let Ok(opt) = elem_peek2.into_option() {
            opt.value()?.into_struct().ok()?
        } else {
            return None;
        }
    };

    // Get the children field and serialize the first child
    if let Ok(children) = elem_struct.field_by_name("children") {
        if let Ok(list) = children.into_list() {
            if let Some(first) = list.get(0) {
                // Serialize this child element to HTML
                return serialize_to_html(first);
            }
        }
    }

    None
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
///
/// Update operations bubble up the tree - we only emit patches for leaf changes:
/// - Text content changed -> SetText
/// - Attribute changed -> SetAttribute
///
/// Element structural updates are NOT translated here because the actual changes
/// are represented by Insert/Delete operations at the leaf level.
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
        // The actual changes are represented by Insert/Delete ops
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
