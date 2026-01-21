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
/// Simple approach: translate each op directly into a patch. No filtering, no deduplication.
/// The ops from cinereus describe how to get from A to B - just apply them.
pub fn translate_to_patches(edit_ops: &[EditOp], new_doc: &Html) -> Vec<Patch> {
    edit_ops
        .iter()
        .filter_map(|op| translate_op(op, new_doc))
        .collect()
}

/// Translate a single EditOp to a DOM Patch.
fn translate_op(op: &EditOp, new_doc: &Html) -> Option<Patch> {
    trace!("translate_op: op={op:?}");
    match op {
        EditOp::Insert { path, value, .. } => translate_insert(&path.0, value.as_deref(), new_doc),
        EditOp::Delete { path, .. } => translate_delete(&path.0),
        EditOp::Update { path, new_value, .. } => translate_update(&path.0, new_value.as_deref()),
        EditOp::Move {
            old_path,
            new_path,
            ..
        } => translate_move(&old_path.0, &new_path.0, new_doc),
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
fn translate_insert(segments: &[PathSegment], value: Option<&str>, new_doc: &Html) -> Option<Patch> {
    let target = path_target(segments);
    let dom_path = to_dom_path(segments);

    trace!("translate_insert: segments={segments:?}, dom_path={dom_path:?}, target={target:?}, value={value:?}");

    match target {
        PathTarget::Element => {
            // For elements, we still need to navigate new_doc to serialize the whole subtree
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
            // Use the value directly from the EditOp
            let attr_value = value?.to_string();
            // DOM path is the element, not the attribute
            let elem_path = to_dom_path(&segments[..segments.len().saturating_sub(2)]);
            Some(Patch::SetAttribute {
                path: NodePath(elem_path),
                name,
                value: attr_value,
            })
        }
        PathTarget::Text => {
            // Use the value directly from the EditOp
            let text = value?.to_string();
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
            // Check if this is an Insert at a structural field - replace inner HTML
            if let Some(PathSegment::Field(name)) = segments.last() {
                // Handle body field - this means the body content changed
                if name == "body" {
                    trace!("handling body insert");
                    let peek = facet::Peek::new(new_doc);
                    if let Some(body_peek) = navigate_peek(peek, segments) {
                        // Unwrap Option<Body>
                        if let Ok(opt) = body_peek.into_option() {
                            if let Some(inner) = opt.value() {
                                if let Ok(s) = inner.into_struct() {
                                    if let Ok(children) = s.field_by_name("children") {
                                        if let Ok(list) = children.into_list() {
                                            let mut children_html = String::new();
                                            for child in list.iter() {
                                                if let Some(html) = serialize_to_html(child) {
                                                    children_html.push_str(&html);
                                                }
                                            }
                                            trace!("body children_html={children_html:?}");
                                            return Some(Patch::ReplaceInnerHtml {
                                                path: NodePath(vec![]),
                                                html: children_html,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if name == "children" {
                    trace!("handling children insert");
                    // Get the parent element's children and serialize them
                    let parent_segments = &segments[..segments.len() - 1];
                    trace!("parent_segments={parent_segments:?}");
                    let peek = facet::Peek::new(new_doc);
                    if let Some(parent_peek) = navigate_peek(peek, parent_segments) {
                        trace!("navigated to parent, shape={:?}", parent_peek.shape());
                        // Handle Option<Body> or similar by unwrapping
                        let struct_peek = if let Ok(opt) = parent_peek.into_option() {
                            opt.value()
                        } else {
                            let peek2 = facet::Peek::new(new_doc);
                            navigate_peek(peek2, parent_segments)
                        };
                        if let Some(struct_peek) = struct_peek {
                            if let Ok(s) = struct_peek.into_struct() {
                                trace!("parent is struct");
                                if let Ok(children) = s.field_by_name("children") {
                                    trace!("got children field, shape={:?}", children.shape());
                                    if let Ok(list) = children.into_list() {
                                        trace!("children is list with {} items", list.len());
                                        let mut children_html = String::new();
                                        for child in list.iter() {
                                            if let Some(html) = serialize_to_html(child) {
                                                children_html.push_str(&html);
                                            }
                                        }
                                        trace!("serialized children_html={children_html:?}");
                                        return Some(Patch::ReplaceInnerHtml {
                                            path: NodePath(dom_path),
                                            html: children_html,
                                        });
                                    } else {
                                        trace!("children is NOT a list");
                                    }
                                } else {
                                    trace!("no children field in struct");
                                }
                            } else {
                                trace!("unwrapped parent is NOT a struct");
                            }
                        } else {
                            trace!("parent Option is None or couldn't unwrap");
                        }
                    } else {
                        trace!("failed to navigate to parent");
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

    trace!("translate_delete: segments={segments:?}");
    trace!("  dom_path={dom_path:?}, target={target:?}");

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

/// Translate a Move operation.
///
/// For attribute field moves (e.g., value moving from id to class field),
/// we translate to SetAttribute or RemoveAttribute based on the new value.
fn translate_move(old_segments: &[PathSegment], new_segments: &[PathSegment], new_doc: &Html) -> Option<Patch> {
    let old_target = path_target(old_segments);
    let new_target = path_target(new_segments);

    trace!("translate_move: old={old_segments:?} -> new={new_segments:?}");
    trace!("  old_target={old_target:?}, new_target={new_target:?}");

    // For attribute moves, look up the value at new_path and emit SetAttribute or RemoveAttribute
    if let PathTarget::Attribute(attr_name) = new_target {
        let elem_path = to_dom_path(&new_segments[..new_segments.len().saturating_sub(2)]);

        // Navigate to the attribute value in new_doc
        let peek = facet::Peek::new(new_doc);
        if let Some(attr_peek) = navigate_peek(peek, new_segments) {
            // Try to get the Option value
            if let Ok(opt) = attr_peek.into_option() {
                if let Some(inner) = opt.value() {
                    // Has a value - SetAttribute
                    if let Some(s) = inner.as_str() {
                        return Some(Patch::SetAttribute {
                            path: NodePath(elem_path),
                            name: attr_name,
                            value: s.to_string(),
                        });
                    }
                } else {
                    // None - RemoveAttribute
                    return Some(Patch::RemoveAttribute {
                        path: NodePath(elem_path),
                        name: attr_name,
                    });
                }
            }
        }
        return None;
    }

    // For element moves, translate to DOM Move
    if old_target == PathTarget::Element && new_target == PathTarget::Element {
        let from = to_dom_path(old_segments);
        let to = to_dom_path(new_segments);
        return Some(Patch::Move {
            from: NodePath(from),
            to: NodePath(to),
        });
    }

    // Other structural moves don't translate directly
    None
}

/// Translate an Update operation.
///
/// Uses the new_value directly from the EditOp.
fn translate_update(segments: &[PathSegment], new_value: Option<&str>) -> Option<Patch> {
    let target = path_target(segments);
    let dom_path = to_dom_path(segments);

    trace!("translate_update: segments={segments:?}");
    trace!("  dom_path={dom_path:?}, target={target:?}, new_value={new_value:?}");

    match target {
        PathTarget::Text => {
            // Use the value directly from the EditOp
            let text = new_value?.to_string();
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
            // Use the value directly from the EditOp
            let value = new_value?.to_string();
            let elem_path = to_dom_path(&segments[..segments.len().saturating_sub(2)]);
            Some(Patch::SetAttribute {
                path: NodePath(elem_path),
                name,
                value,
            })
        }
        PathTarget::Element | PathTarget::Other => {
            // Structural updates don't translate to DOM patches directly
            // The leaf changes (text, attributes) are what matter
            None
        }
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

