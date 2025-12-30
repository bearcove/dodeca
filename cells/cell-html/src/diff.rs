//! HTML diffing using facet-html and facet-diff
//!
//! Translates facet-diff EditOps into DOM Patches for live reload.

use cell_html_proto::Patch;
use dodeca_protocol::NodePath;
use facet_diff::{EditOp, PathSegment, tree_diff};
use facet_html::{self as html, elements::*};

/// Diff two HTML strings and return patches
pub fn diff_html(old_html: &str, new_html: &str) -> Result<Vec<Patch>, String> {
    let old_doc: Html = html::from_str(old_html).map_err(|e| format!("parse old: {e}"))?;
    let new_doc: Html = html::from_str(new_html).map_err(|e| format!("parse new: {e}"))?;

    let edit_ops = tree_diff(&old_doc, &new_doc);

    Ok(translate_to_patches(&edit_ops, &new_doc, new_html))
}

/// Translate facet-diff EditOps into DOM Patches.
fn translate_to_patches(edit_ops: &[EditOp], new_doc: &Html, new_html: &str) -> Vec<Patch> {
    let mut patches = Vec::new();

    // First pass: look for paired Insert+Delete on same children array (text change pattern)
    let text_changes = detect_text_changes(edit_ops, new_doc);
    patches.extend(text_changes);

    // Second pass: look for attribute changes
    let attr_changes = detect_attr_changes(edit_ops, new_doc);
    patches.extend(attr_changes);

    // Third pass: handle remaining operations
    for op in edit_ops {
        if let Some(patch) = translate_edit_op(op, new_doc, new_html) {
            // Deduplicate: don't add if we already have a patch at the same or parent path
            if !is_redundant(&patches, &patch) {
                patches.push(patch);
            }
        }
    }

    patches
}

/// Detect attribute changes (Insert+Delete pairs on attrs.X or direct field attributes like href)
fn detect_attr_changes(edit_ops: &[EditOp], new_doc: &Html) -> Vec<Patch> {
    let mut patches = Vec::new();

    // Known direct attributes (not inside attrs struct)
    let direct_attrs = [
        "href",
        "src",
        "alt",
        "target",
        "rel",
        "download",
        "type",
        "action",
        "method",
        "name",
        "value",
        "placeholder",
    ];

    for op in edit_ops {
        if let EditOp::Insert { path, .. } = op {
            let segments = &path.0;

            if segments.len() >= 2
                && let (Some(PathSegment::Field(field)), Some(PathSegment::Field(attr_name))) =
                    (segments.get(segments.len() - 2), segments.last())
                && field == "attrs"
                && let Some(patch) = try_attr_change(edit_ops, new_doc, segments, attr_name)
            {
                patches.push(patch);
            }

            // Check if path ends with a direct attribute field
            if let Some(PathSegment::Field(attr_name)) = segments.last()
                && direct_attrs.contains(&attr_name.as_ref())
                && let Some(patch) = try_attr_change(edit_ops, new_doc, segments, attr_name)
            {
                patches.push(patch);
            }
        }
    }

    patches
}

fn try_attr_change(
    edit_ops: &[EditOp],
    new_doc: &Html,
    segments: &[PathSegment],
    attr_name: &str,
) -> Option<Patch> {
    // Check if there's a matching Delete
    let has_matching_delete = edit_ops.iter().any(|other| {
        if let EditOp::Delete { path: del_path, .. } = other {
            del_path.0 == segments
        } else {
            false
        }
    });

    if !has_matching_delete {
        return None;
    }

    // This is an attribute change
    let dom_path = extract_dom_path(segments);
    let value = get_attr_value_at_path(new_doc, segments, attr_name)?;

    Some(Patch::SetAttribute {
        path: NodePath(dom_path),
        name: attr_name.to_string(),
        value,
    })
}

/// Get attribute value at a specific path
fn get_attr_value_at_path(doc: &Html, segments: &[PathSegment], attr_name: &str) -> Option<String> {
    let body = doc.body.as_ref()?;

    let mut seg_iter = segments.iter().peekable();

    // Skip "body"
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "body" => {}
        _ => return None,
    }

    // Skip "children"
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "children" => {}
        _ => return None,
    }

    get_attr_from_flow_content_nav(&body.children, &mut seg_iter, attr_name)
}

fn get_attr_from_flow_content_nav<'a>(
    children: &[FlowContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
    attr_name: &str,
) -> Option<String> {
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    // Skip variant
    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    // Skip tuple index
    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    // Check what's next
    match seg_iter.next() {
        Some(PathSegment::Field(f)) if f == "attrs" => {
            // We're at the attrs - get the attribute value
            get_attr_from_element(child, attr_name)
        }
        Some(PathSegment::Field(f)) if f == "children" => {
            // Navigate deeper
            match child {
                FlowContent::Div(d) => {
                    get_attr_from_flow_content_nav(&d.children, seg_iter, attr_name)
                }
                FlowContent::P(p) => {
                    get_attr_from_phrasing_content_nav(&p.children, seg_iter, attr_name)
                }
                _ => None,
            }
        }
        Some(PathSegment::Field(f)) => {
            // Direct attribute field (like href on A)
            get_direct_attr_from_element(child, f)
        }
        _ => None,
    }
}

fn get_direct_attr_from_element(elem: &FlowContent, attr_name: &str) -> Option<String> {
    match elem {
        FlowContent::A(a) => match attr_name {
            "href" => a.href.clone(),
            "target" => a.target.clone(),
            "rel" => a.rel.clone(),
            _ => None,
        },
        FlowContent::Img(img) => match attr_name {
            "src" => img.src.clone(),
            "alt" => img.alt.clone(),
            _ => None,
        },
        _ => None,
    }
}

fn get_attr_from_phrasing_content_nav<'a>(
    children: &[PhrasingContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
    attr_name: &str,
) -> Option<String> {
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    match seg_iter.next() {
        Some(PathSegment::Field(f)) if f == "attrs" => {
            get_attr_from_phrasing_element(child, attr_name)
        }
        Some(PathSegment::Field(f)) if f == "children" => match child {
            PhrasingContent::Span(s) => {
                get_attr_from_phrasing_content_nav(&s.children, seg_iter, attr_name)
            }
            PhrasingContent::Strong(s) => {
                get_attr_from_phrasing_content_nav(&s.children, seg_iter, attr_name)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Detect text changes (Insert+Delete pairs on children arrays)
fn detect_text_changes(edit_ops: &[EditOp], new_doc: &Html) -> Vec<Patch> {
    let mut patches = Vec::new();

    // Find Insert operations on children arrays
    for op in edit_ops {
        if let EditOp::Insert { path, .. } = op {
            let segments = &path.0;

            // Check if path ends with .children (not .children.[n])
            if let Some(PathSegment::Field(f)) = segments.last()
                && f == "children"
            {
                // This is an insert into a children array
                // Check if there's a matching Delete (indicating replacement, not addition)
                let has_matching_delete = edit_ops.iter().any(|other| {
                    if let EditOp::Delete { path: del_path, .. } = other {
                        del_path.0 == path.0
                    } else {
                        false
                    }
                });

                if has_matching_delete {
                    // This is a text change (or element swap)
                    // Try to get the new text content
                    if let Some(text) = get_text_from_children(new_doc, segments) {
                        let dom_path = extract_dom_path(segments);
                        if !dom_path.is_empty() {
                            patches.push(Patch::SetText {
                                path: NodePath(dom_path),
                                text,
                            });
                        }
                    }
                }
            }
        }
    }

    patches
}

/// Get text content from a children array path
fn get_text_from_children(doc: &Html, segments: &[PathSegment]) -> Option<String> {
    let body = doc.body.as_ref()?;

    let mut seg_iter = segments.iter().peekable();

    // Skip "body"
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "body" => {}
        _ => return None,
    }

    // Skip first "children"
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "children" => {}
        _ => return None,
    }

    // Navigate to the element containing the children
    navigate_to_element_text(&body.children, &mut seg_iter)
}

fn navigate_to_element_text<'a>(
    children: &[FlowContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
) -> Option<String> {
    // Get index
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    // Skip variant
    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    // Skip tuple index
    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    // Check if next is "children" - if it's the last thing, get text from this element
    match seg_iter.next() {
        Some(PathSegment::Field(f)) if f == "children" => {
            // Check if this is the end
            if seg_iter.peek().is_none() {
                // This element's children changed - get its text content
                return get_element_text_content(child);
            }
            // Otherwise navigate deeper
            match child {
                FlowContent::Div(d) => navigate_to_element_text(&d.children, seg_iter),
                FlowContent::P(p) => navigate_to_phrasing_text(&p.children, seg_iter),
                FlowContent::H1(h) => navigate_to_phrasing_text(&h.children, seg_iter),
                FlowContent::H2(h) => navigate_to_phrasing_text(&h.children, seg_iter),
                FlowContent::H3(h) => navigate_to_phrasing_text(&h.children, seg_iter),
                _ => None,
            }
        }
        _ => None,
    }
}

fn navigate_to_phrasing_text<'a>(
    children: &[PhrasingContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
) -> Option<String> {
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    match seg_iter.next() {
        Some(PathSegment::Field(f)) if f == "children" => {
            if seg_iter.peek().is_none() {
                return get_phrasing_element_text_content(child);
            }
            match child {
                PhrasingContent::Strong(s) => navigate_to_phrasing_text(&s.children, seg_iter),
                PhrasingContent::Em(e) => navigate_to_phrasing_text(&e.children, seg_iter),
                PhrasingContent::Span(s) => navigate_to_phrasing_text(&s.children, seg_iter),
                _ => None,
            }
        }
        _ => None,
    }
}

fn get_element_text_content(elem: &FlowContent) -> Option<String> {
    match elem {
        FlowContent::P(p) => collect_phrasing_text(&p.children),
        FlowContent::H1(h) => collect_phrasing_text(&h.children),
        FlowContent::H2(h) => collect_phrasing_text(&h.children),
        FlowContent::H3(h) => collect_phrasing_text(&h.children),
        FlowContent::H4(h) => collect_phrasing_text(&h.children),
        FlowContent::H5(h) => collect_phrasing_text(&h.children),
        FlowContent::H6(h) => collect_phrasing_text(&h.children),
        FlowContent::Span(s) => collect_phrasing_text(&s.children),
        FlowContent::Strong(s) => collect_phrasing_text(&s.children),
        FlowContent::Em(e) => collect_phrasing_text(&e.children),
        FlowContent::Text(t) => Some(t.clone()),
        _ => None,
    }
}

fn get_phrasing_element_text_content(elem: &PhrasingContent) -> Option<String> {
    match elem {
        PhrasingContent::Text(t) => Some(t.clone()),
        PhrasingContent::Span(s) => collect_phrasing_text(&s.children),
        PhrasingContent::Strong(s) => collect_phrasing_text(&s.children),
        PhrasingContent::Em(e) => collect_phrasing_text(&e.children),
        PhrasingContent::A(a) => collect_phrasing_text(&a.children),
        PhrasingContent::Code(c) => collect_phrasing_text(&c.children),
        _ => None,
    }
}

fn collect_phrasing_text(children: &[PhrasingContent]) -> Option<String> {
    // For simple case: single text child
    if children.len() == 1
        && let PhrasingContent::Text(t) = &children[0]
    {
        return Some(t.clone());
    }
    // For mixed content, we can't easily do SetText - return None to fall back to Replace
    None
}

/// Extract DOM path from segments (for the element, not the children array)
fn extract_dom_path(segments: &[PathSegment]) -> Vec<usize> {
    let mut dom_path = Vec::new();
    let mut in_body = false;

    let mut i = 0;
    while i < segments.len() {
        match &segments[i] {
            PathSegment::Field(name) if name == "body" => {
                in_body = true;
            }
            PathSegment::Field(name) if name == "children" && in_body => {
                // Next should be index
                if let Some(PathSegment::Index(idx)) = segments.get(i + 1) {
                    dom_path.push(*idx);
                    i += 1;
                }
            }
            PathSegment::Variant(_) => {
                // Skip variant + tuple index
                if let Some(PathSegment::Index(_)) = segments.get(i + 1) {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    dom_path
}

/// Check if a patch is redundant given existing patches
fn is_redundant(existing: &[Patch], new_patch: &Patch) -> bool {
    let new_path = patch_path(new_patch);

    for existing_patch in existing {
        let existing_path = patch_path(existing_patch);

        // If existing patch is at same path or is an ancestor, new patch is redundant
        if is_same_or_ancestor(existing_path, new_path) {
            return true;
        }

        // Also: if new patch is an ancestor of existing, it's redundant (existing is more specific)
        if is_same_or_ancestor(new_path, existing_path) {
            return true;
        }
    }

    false
}

fn patch_path(patch: &Patch) -> &[usize] {
    match patch {
        Patch::SetText { path, .. } => &path.0,
        Patch::SetAttribute { path, .. } => &path.0,
        Patch::RemoveAttribute { path, .. } => &path.0,
        Patch::Remove { path } => &path.0,
        Patch::Replace { path, .. } => &path.0,
        Patch::InsertBefore { path, .. } => &path.0,
        Patch::InsertAfter { path, .. } => &path.0,
        Patch::AppendChild { path, .. } => &path.0,
    }
}

fn is_same_or_ancestor(ancestor: &[usize], descendant: &[usize]) -> bool {
    if ancestor.len() > descendant.len() {
        return false;
    }
    ancestor == &descendant[..ancestor.len()]
}

/// Translate a single EditOp to a Patch
fn translate_edit_op(op: &EditOp, new_doc: &Html, new_html: &str) -> Option<Patch> {
    match op {
        EditOp::Update { path, .. } => translate_update(&path.0, new_doc, new_html),
        EditOp::Insert { path, .. } => translate_insert(&path.0, new_doc, new_html),
        EditOp::Delete { path, .. } => translate_delete(&path.0),
        EditOp::Move { .. } => {
            // Move is complex - for now skip (will be handled by parent replace)
            None
        }
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Translate an Update operation
fn translate_update(segments: &[PathSegment], new_doc: &Html, new_html: &str) -> Option<Patch> {
    // Analyze what kind of update this is
    let analysis = analyze_path(segments);

    match analysis.target {
        PathTarget::Text => {
            // Text content changed - need SetText at the parent element
            if analysis.dom_path.is_empty() {
                return None; // Can't set text on body itself
            }

            // Get the new text value by navigating the new document
            if let Some(text) = get_text_at_path(new_doc, segments) {
                Some(Patch::SetText {
                    path: NodePath(analysis.dom_path[..analysis.dom_path.len() - 1].to_vec()),
                    text,
                })
            } else {
                // Fall back to replace
                replace_at_path(&analysis.dom_path, new_html)
            }
        }
        PathTarget::Attribute(name) => {
            // Attribute changed
            if let Some(value) = get_attribute_at_path(new_doc, segments, &name) {
                Some(Patch::SetAttribute {
                    path: NodePath(analysis.dom_path),
                    name,
                    value,
                })
            } else {
                // Attribute removed
                Some(Patch::RemoveAttribute {
                    path: NodePath(analysis.dom_path),
                    name,
                })
            }
        }
        PathTarget::Element => {
            // Element itself changed (structural) - replace it
            replace_at_path(&analysis.dom_path, new_html)
        }
        PathTarget::Unknown => None,
    }
}

/// Translate an Insert operation
fn translate_insert(segments: &[PathSegment], _new_doc: &Html, new_html: &str) -> Option<Patch> {
    let analysis = analyze_path(segments);

    // For inserts, we need to get the HTML of the inserted element
    // and insert it at the right position
    // For now, fall back to replacing the parent
    if analysis.dom_path.is_empty() {
        replace_at_path(&[], new_html)
    } else {
        replace_at_path(&analysis.dom_path[..analysis.dom_path.len() - 1], new_html)
    }
}

/// Translate a Delete operation
fn translate_delete(segments: &[PathSegment]) -> Option<Patch> {
    let analysis = analyze_path(segments);

    if analysis.dom_path.is_empty() {
        return None; // Can't delete body
    }

    Some(Patch::Remove {
        path: NodePath(analysis.dom_path),
    })
}

/// What the path points to
#[derive(Debug, Clone, PartialEq)]
enum PathTarget {
    Text,
    Attribute(String),
    Element,
    Unknown,
}

/// Result of analyzing a facet path
#[derive(Debug)]
struct PathAnalysis {
    /// The DOM path (indices into childNodes)
    dom_path: Vec<usize>,
    /// What the path targets
    target: PathTarget,
}

/// Analyze a facet path to extract DOM path and target type
fn analyze_path(segments: &[PathSegment]) -> PathAnalysis {
    let mut dom_path = Vec::new();
    let mut target = PathTarget::Unknown;
    let mut in_body = false;

    let mut i = 0;
    while i < segments.len() {
        match &segments[i] {
            PathSegment::Field(name) if name == "body" => {
                in_body = true;
            }
            PathSegment::Field(name) if name == "children" && in_body => {
                // Next segment should be an index
                if let Some(PathSegment::Index(idx)) = segments.get(i + 1) {
                    dom_path.push(*idx);
                    i += 1;
                    target = PathTarget::Element;
                }
            }
            PathSegment::Field(name) if name == "attrs" => {
                // Next segment is the attribute name
                if let Some(PathSegment::Field(attr_name)) = segments.get(i + 1) {
                    target = PathTarget::Attribute(attr_name.to_string());
                    i += 1;
                }
            }
            PathSegment::Field(name) if is_known_attribute(name) && in_body => {
                // Direct attribute (flattened from attrs)
                target = PathTarget::Attribute(name.to_string());
            }
            PathSegment::Variant(name) => {
                // Element variant (::div, ::p, ::_text, etc.)
                if name == "_text" || name == "Text" {
                    target = PathTarget::Text;
                } else {
                    target = PathTarget::Element;
                }
                // Skip the tuple index that follows
                if let Some(PathSegment::Index(_)) = segments.get(i + 1) {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    PathAnalysis { dom_path, target }
}

fn is_known_attribute(name: &str) -> bool {
    matches!(
        name,
        "id" | "class"
            | "style"
            | "href"
            | "src"
            | "alt"
            | "title"
            | "name"
            | "value"
            | "type"
            | "rel"
            | "target"
            | "placeholder"
            | "disabled"
            | "checked"
            | "selected"
            | "width"
            | "height"
            | "lang"
            | "dir"
    )
}

/// Get text value at a path in the document
fn get_text_at_path(doc: &Html, segments: &[PathSegment]) -> Option<String> {
    let body = doc.body.as_ref()?;

    let mut seg_iter = segments.iter().peekable();

    // Skip "body" field
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "body" => {}
        _ => return None,
    }

    // Skip "children" field
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "children" => {}
        _ => return None,
    }

    // Navigate through children
    navigate_flow_content(&body.children, &mut seg_iter)
}

fn navigate_flow_content<'a>(
    children: &[FlowContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
) -> Option<String> {
    // Get the index
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    // Skip variant tag
    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    // Skip tuple index
    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    // Check if this is a text node
    if let FlowContent::Text(text) = child {
        return Some(text.clone());
    }

    // Check if next is "children" (nested element)
    match seg_iter.peek() {
        Some(PathSegment::Field(f)) if *f == "children" => {
            seg_iter.next(); // consume "children"
            // Navigate into nested children
            match child {
                FlowContent::P(p) => navigate_phrasing_content(&p.children, seg_iter),
                FlowContent::Div(d) => navigate_flow_content(&d.children, seg_iter),
                FlowContent::H1(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::H2(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::H3(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::H4(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::H5(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::H6(h) => navigate_phrasing_content(&h.children, seg_iter),
                FlowContent::Span(s) => navigate_phrasing_content(&s.children, seg_iter),
                FlowContent::Strong(s) => navigate_phrasing_content(&s.children, seg_iter),
                FlowContent::Em(e) => navigate_phrasing_content(&e.children, seg_iter),
                _ => None,
            }
        }
        _ => None,
    }
}

fn navigate_phrasing_content<'a>(
    children: &[PhrasingContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
) -> Option<String> {
    // Get the index
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    // Skip variant tag
    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    // Skip tuple index
    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    // Check if this is a text node
    if let PhrasingContent::Text(text) = child {
        return Some(text.clone());
    }

    // Check if next is "children" (nested element)
    match seg_iter.peek() {
        Some(PathSegment::Field(f)) if *f == "children" => {
            seg_iter.next(); // consume "children"
            // Navigate into nested children
            match child {
                PhrasingContent::Span(s) => navigate_phrasing_content(&s.children, seg_iter),
                PhrasingContent::Strong(s) => navigate_phrasing_content(&s.children, seg_iter),
                PhrasingContent::Em(e) => navigate_phrasing_content(&e.children, seg_iter),
                PhrasingContent::A(a) => navigate_phrasing_content(&a.children, seg_iter),
                PhrasingContent::Code(c) => navigate_phrasing_content(&c.children, seg_iter),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Get attribute value at a path in the document
fn get_attribute_at_path(doc: &Html, segments: &[PathSegment], attr: &str) -> Option<String> {
    let body = doc.body.as_ref()?;

    let mut seg_iter = segments.iter().peekable();

    // Skip "body" field
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "body" => {}
        _ => return None,
    }

    // Skip "children" field
    match seg_iter.next()? {
        PathSegment::Field(f) if f == "children" => {}
        _ => return None,
    }

    get_attr_from_flow_content(&body.children, &mut seg_iter, attr)
}

fn get_attr_from_flow_content<'a>(
    children: &[FlowContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
    attr: &str,
) -> Option<String> {
    // Get the index
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    // Skip variant tag
    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    // Skip tuple index
    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    // Check if we're at the target (next is "attrs" or direct attr name)
    match seg_iter.peek() {
        Some(PathSegment::Field(f)) if *f == "attrs" => {
            // We're at the element - get the attribute
            get_attr_from_element(child, attr)
        }
        Some(PathSegment::Field(f)) if is_known_attribute(f) => {
            // Direct attribute (flattened)
            get_attr_from_element(child, attr)
        }
        Some(PathSegment::Field(f)) if *f == "children" => {
            seg_iter.next(); // consume "children"
            // Navigate deeper
            match child {
                FlowContent::Div(d) => get_attr_from_flow_content(&d.children, seg_iter, attr),
                FlowContent::P(p) => get_attr_from_phrasing_content(&p.children, seg_iter, attr),
                _ => None,
            }
        }
        _ => None,
    }
}

fn get_attr_from_phrasing_content<'a>(
    children: &[PhrasingContent],
    seg_iter: &mut std::iter::Peekable<impl Iterator<Item = &'a PathSegment>>,
    attr: &str,
) -> Option<String> {
    let idx = match seg_iter.next()? {
        PathSegment::Index(i) => *i,
        _ => return None,
    };

    let child = children.get(idx)?;

    match seg_iter.next()? {
        PathSegment::Variant(_) => {}
        _ => return None,
    }

    match seg_iter.next()? {
        PathSegment::Index(0) => {}
        _ => return None,
    }

    match seg_iter.peek() {
        Some(PathSegment::Field(f)) if *f == "attrs" || is_known_attribute(f) => {
            get_attr_from_phrasing_element(child, attr)
        }
        Some(PathSegment::Field(f)) if *f == "children" => {
            seg_iter.next();
            match child {
                PhrasingContent::Span(s) => {
                    get_attr_from_phrasing_content(&s.children, seg_iter, attr)
                }
                PhrasingContent::Strong(s) => {
                    get_attr_from_phrasing_content(&s.children, seg_iter, attr)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn get_attr_from_element(elem: &FlowContent, attr: &str) -> Option<String> {
    let attrs = match elem {
        FlowContent::Div(d) => &d.attrs,
        FlowContent::P(p) => &p.attrs,
        FlowContent::H1(h) => &h.attrs,
        FlowContent::H2(h) => &h.attrs,
        FlowContent::H3(h) => &h.attrs,
        FlowContent::Span(s) => &s.attrs,
        FlowContent::Strong(s) => &s.attrs,
        FlowContent::Em(e) => &e.attrs,
        _ => return None,
    };

    match attr {
        "id" => attrs.id.clone(),
        "class" => attrs.class.clone(),
        "style" => attrs.style.clone(),
        "title" => attrs.tooltip.clone(),
        _ => None,
    }
}

fn get_attr_from_phrasing_element(elem: &PhrasingContent, attr: &str) -> Option<String> {
    let attrs = match elem {
        PhrasingContent::Span(s) => &s.attrs,
        PhrasingContent::Strong(s) => &s.attrs,
        PhrasingContent::Em(e) => &e.attrs,
        PhrasingContent::A(a) => &a.attrs,
        PhrasingContent::Code(c) => &c.attrs,
        _ => return None,
    };

    match attr {
        "id" => attrs.id.clone(),
        "class" => attrs.class.clone(),
        "style" => attrs.style.clone(),
        "title" => attrs.tooltip.clone(),
        _ => None,
    }
}

/// Create a Replace patch at the given DOM path
fn replace_at_path(dom_path: &[usize], new_html: &str) -> Option<Patch> {
    // Extract the element HTML at the path from new_html
    // For now, if path is empty, extract the body
    if dom_path.is_empty() {
        extract_body_html(new_html).map(|html| Patch::Replace {
            path: NodePath(vec![]),
            html,
        })
    } else {
        // TODO: Extract specific element at path
        // For now, fall back to body replace
        extract_body_html(new_html).map(|html| Patch::Replace {
            path: NodePath(vec![]),
            html,
        })
    }
}

/// Extract body HTML from full document
fn extract_body_html(html: &str) -> Option<String> {
    let body_start = html.find("<body")?;
    let body_end = html.rfind("</body>")?;
    Some(html[body_start..body_end + 7].to_string())
}
