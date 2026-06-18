//! SVG optimization passes.

use std::collections::HashSet;

use crate::Options;
use crate::ast::*;
use crate::path::{parse_path, serialize_path};

/// Apply all enabled optimizations to the document.
pub fn optimize(doc: &mut Document, options: &Options) {
    if options.remove_metadata {
        remove_metadata(&mut doc.root);
    }

    if options.remove_unused_namespaces {
        remove_unused_namespaces(&mut doc.root);
    }

    if options.remove_comments {
        remove_comments(&mut doc.root);
    }

    if options.remove_hidden {
        remove_hidden(&mut doc.root);
    }

    if options.remove_empty {
        remove_empty(&mut doc.root);
    }

    if options.collapse_groups {
        collapse_groups(&mut doc.root);
    }

    if options.minify_paths {
        minify_paths(&mut doc.root, options.precision);
    }

    if options.minify_colors {
        minify_colors(&mut doc.root);
    }

    if options.remove_defaults {
        remove_default_attrs(&mut doc.root);
    }

    if options.minify_styles {
        minify_styles(&mut doc.root);
    }

    // Clean up whitespace-only text nodes
    cleanup_whitespace(&mut doc.root);
}

/// Remove metadata, title, desc, and other non-rendering elements.
fn remove_metadata(elem: &mut Element) {
    let metadata_elements = ["metadata", "title", "desc"];

    elem.children.retain(|node| {
        if let Node::Element(e) = node {
            let full_name = e.name.full_name();
            !metadata_elements.iter().any(|&name| e.name.local == name)
                && !full_name.starts_with("sodipodi:")
                && !full_name.starts_with("inkscape:")
                && e.name.prefix.as_deref() != Some("sodipodi")
                && e.name.prefix.as_deref() != Some("inkscape")
        } else {
            true
        }
    });

    // Remove editor-specific attributes
    elem.attributes.retain(|attr| {
        let name = &attr.name;
        !name.full_name().starts_with("sodipodi:")
            && !name.full_name().starts_with("inkscape:")
            && name.local != "data-name"
            && (name.local != "id" || is_id_referenced(&attr.value))
    });

    for child in elem.child_elements_mut() {
        remove_metadata(child);
    }
}

fn is_id_referenced(_id: &str) -> bool {
    // TODO: track ID references (url(#id), xlink:href="#id", etc.)
    // For now, keep all IDs to be safe
    true
}

/// Remove unused namespace declarations.
fn remove_unused_namespaces(elem: &mut Element) {
    // Collect all prefixes actually used in the document
    let mut used_prefixes: HashSet<Option<String>> = HashSet::new();
    collect_used_prefixes(elem, &mut used_prefixes);

    // Remove unused xmlns declarations
    elem.attributes.retain(|attr| {
        if attr.name.local == "xmlns" && attr.name.prefix.is_none() {
            // Default namespace - always keep
            true
        } else if attr.name.prefix.as_deref() == Some("xmlns") {
            // xmlns:prefix - keep if prefix is used
            used_prefixes.contains(&Some(attr.name.local.clone()))
        } else {
            true
        }
    });
}

fn collect_used_prefixes(elem: &Element, used: &mut HashSet<Option<String>>) {
    // Element prefix
    used.insert(elem.name.prefix.clone());

    // Attribute prefixes
    for attr in &elem.attributes {
        if attr.name.prefix.is_some() && !attr.name.is_xmlns() {
            used.insert(attr.name.prefix.clone());
        }
    }

    // Recurse
    for child in elem.child_elements() {
        collect_used_prefixes(child, used);
    }
}

/// Remove comment nodes.
fn remove_comments(elem: &mut Element) {
    elem.children
        .retain(|node| !matches!(node, Node::Comment(_)));

    for child in elem.child_elements_mut() {
        remove_comments(child);
    }
}

/// Remove hidden elements (display:none, visibility:hidden, opacity:0).
fn remove_hidden(elem: &mut Element) {
    elem.children.retain(|node| {
        if let Node::Element(e) = node {
            !is_hidden(e)
        } else {
            true
        }
    });

    for child in elem.child_elements_mut() {
        remove_hidden(child);
    }
}

fn is_hidden(elem: &Element) -> bool {
    // Check display attribute
    if elem.get_attr("display") == Some("none") {
        return true;
    }

    // Check visibility attribute
    if elem.get_attr("visibility") == Some("hidden") {
        return true;
    }

    // Check opacity
    if let Some(opacity) = elem.get_attr("opacity")
        && opacity.parse::<f64>().ok() == Some(0.0)
    {
        return true;
    }

    // Check style attribute for display:none
    if let Some(style) = elem.get_attr("style")
        && (style.contains("display:none") || style.contains("display: none"))
    {
        return true;
    }

    false
}

/// Remove empty container elements.
fn remove_empty(elem: &mut Element) {
    // First recurse
    for child in elem.child_elements_mut() {
        remove_empty(child);
    }

    // Then remove empty containers
    let container_elements = [
        "g", "defs", "symbol", "marker", "clipPath", "mask", "pattern",
    ];

    elem.children.retain(|node| {
        if let Node::Element(e) = node {
            if container_elements.contains(&e.name.local.as_str()) {
                // Keep if it has children or important attributes
                !e.children.is_empty() || e.get_attr("id").is_some()
            } else {
                true
            }
        } else {
            true
        }
    });
}

/// Collapse groups that serve no purpose.
fn collapse_groups(elem: &mut Element) {
    // First recurse
    for child in elem.child_elements_mut() {
        collapse_groups(child);
    }

    // Collect indices of groups we can collapse
    let mut new_children = Vec::new();

    for child in std::mem::take(&mut elem.children) {
        if let Node::Element(e) = &child {
            if can_collapse_group(e) {
                // Collapse: add the group's children directly
                if let Node::Element(mut e) = child {
                    new_children.extend(std::mem::take(&mut e.children));
                }
            } else {
                new_children.push(child);
            }
        } else {
            new_children.push(child);
        }
    }

    elem.children = new_children;
}

fn can_collapse_group(elem: &Element) -> bool {
    // Only collapse <g> elements
    if elem.name.local != "g" {
        return false;
    }

    // Don't collapse if it has an id (might be referenced)
    if elem.get_attr("id").is_some() {
        return false;
    }

    // Don't collapse if it has any meaningful attributes
    let dominated_attrs = ["class", "style", "transform", "fill", "stroke", "opacity"];
    for attr in &elem.attributes {
        if dominated_attrs.contains(&attr.name.local.as_str()) {
            return false;
        }
    }

    // Don't collapse if it has multiple children (preserve structure)
    elem.children.len() == 1
}

/// Minify path data.
fn minify_paths(elem: &mut Element, precision: u8) {
    if elem.name.local == "path"
        && let Some(d) = elem.get_attr("d").map(|s| s.to_string())
        && let Ok(path) = parse_path(&d)
    {
        let minified = serialize_path(&path, precision);
        elem.set_attr("d", minified);
    }

    for child in elem.child_elements_mut() {
        minify_paths(child, precision);
    }
}

/// Minify color values.
fn minify_colors(elem: &mut Element) {
    let color_attrs = [
        "fill",
        "stroke",
        "stop-color",
        "flood-color",
        "lighting-color",
        "color",
    ];

    for attr in &mut elem.attributes {
        if color_attrs.contains(&attr.name.local.as_str()) {
            attr.value = minify_color(&attr.value);
        }
    }

    // Also check style attribute
    if let Some(style) = elem.get_attr("style").map(|s| s.to_string()) {
        let new_style = minify_style_colors(&style);
        elem.set_attr("style", new_style);
    }

    for child in elem.child_elements_mut() {
        minify_colors(child);
    }
}

fn minify_color(color: &str) -> String {
    let color = color.trim();
    let lower = color.to_lowercase();

    // Check for named color shortcuts first
    match lower.as_str() {
        "white" | "#ffffff" | "#fff" => return "#fff".into(),
        "black" | "#000000" | "#000" => return "#000".into(),
        "#ff0000" | "#f00" => return "red".into(),
        "#0000ff" | "#00f" => return "blue".into(),
        "red" => return "red".into(),
        "blue" => return "blue".into(),
        _ => {}
    }

    // #RRGGBB -> #RGB if possible
    if color.len() == 7 && color.starts_with('#') {
        let hex = &lower[1..];
        let bytes: Vec<u8> = (0..6)
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect();

        if bytes.len() == 3 {
            let (r, g, b) = (bytes[0], bytes[1], bytes[2]);
            if r >> 4 == r & 0xf && g >> 4 == g & 0xf && b >> 4 == b & 0xf {
                return format!("#{:x}{:x}{:x}", r & 0xf, g & 0xf, b & 0xf);
            }
        }
    }

    color.to_string()
}

fn minify_style_colors(style: &str) -> String {
    let mut result = String::new();
    for decl in style.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }

        if !result.is_empty() {
            result.push(';');
        }

        if let Some((prop, value)) = decl.split_once(':') {
            let prop = prop.trim();
            let value = value.trim();
            result.push_str(prop);
            result.push(':');
            if [
                "fill",
                "stroke",
                "color",
                "stop-color",
                "flood-color",
                "lighting-color",
            ]
            .contains(&prop)
            {
                result.push_str(&minify_color(value));
            } else {
                result.push_str(value);
            }
        } else {
            result.push_str(decl);
        }
    }
    result
}

/// Remove default attribute values.
fn remove_default_attrs(elem: &mut Element) {
    elem.attributes
        .retain(|attr| !is_default_value(&elem.name.local, &attr.name.local, &attr.value));

    for child in elem.child_elements_mut() {
        remove_default_attrs(child);
    }
}

fn is_default_value(element: &str, attr: &str, value: &str) -> bool {
    // Common defaults
    match (element, attr, value) {
        // SVG element defaults
        (_, "version", "1.1") => true,
        (_, "baseProfile", "full") => true,
        (_, "preserveAspectRatio", "xMidYMid meet") => true,

        // Presentation attribute defaults
        (_, "fill-opacity", "1") => true,
        (_, "stroke-opacity", "1") => true,
        (_, "opacity", "1") => true,
        (_, "stroke-width", "1") => true,
        (_, "stroke-linecap", "butt") => true,
        (_, "stroke-linejoin", "miter") => true,
        (_, "stroke-miterlimit", "4") => true,
        (_, "fill-rule", "nonzero") => true,
        (_, "clip-rule", "nonzero") => true,
        (_, "font-style", "normal") => true,
        (_, "font-weight", "normal") | (_, "font-weight", "400") => true,
        (_, "text-anchor", "start") => true,
        (_, "dominant-baseline", "auto") => true,
        (_, "visibility", "visible") => true,
        (_, "display", "inline") => true,
        (_, "overflow", "visible") => true,

        // Specific element defaults
        ("rect", "rx", "0") | ("rect", "ry", "0") => true,
        ("circle", "cx", "0") | ("circle", "cy", "0") => true,
        ("ellipse", "cx", "0") | ("ellipse", "cy", "0") => true,
        ("line", "x1", "0") | ("line", "y1", "0") | ("line", "x2", "0") | ("line", "y2", "0") => {
            true
        }

        _ => false,
    }
}

/// Minify inline styles.
fn minify_styles(elem: &mut Element) {
    if let Some(style) = elem.get_attr("style").map(|s| s.to_string()) {
        let minified = minify_style(&style);
        if minified.is_empty() {
            elem.remove_attr("style");
        } else {
            elem.set_attr("style", minified);
        }
    }

    for child in elem.child_elements_mut() {
        minify_styles(child);
    }
}

fn minify_style(style: &str) -> String {
    let mut parts = Vec::new();

    for decl in style.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }

        if let Some((prop, value)) = decl.split_once(':') {
            let prop = prop.trim();
            let value = value.trim();

            // Skip default values
            if is_default_style_value(prop, value) {
                continue;
            }

            parts.push(format!("{}:{}", prop, value));
        }
    }

    parts.join(";")
}

fn is_default_style_value(prop: &str, value: &str) -> bool {
    matches!(
        (prop, value),
        ("fill-opacity", "1")
            | ("stroke-opacity", "1")
            | ("opacity", "1")
            | ("stroke-width", "1")
            | ("font-style", "normal")
            | ("font-weight", "normal")
            | ("font-weight", "400")
    )
}

/// Clean up whitespace-only text nodes.
fn cleanup_whitespace(elem: &mut Element) {
    elem.children.retain(|node| {
        if let Node::Text(text) = node {
            !text.trim().is_empty()
        } else {
            true
        }
    });

    for child in elem.child_elements_mut() {
        cleanup_whitespace(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_color() {
        assert_eq!(minify_color("#ffffff"), "#fff");
        assert_eq!(minify_color("#ff0000"), "red");
        assert_eq!(minify_color("#aabbcc"), "#abc");
        assert_eq!(minify_color("#abcdef"), "#abcdef"); // can't shorten
    }

    #[test]
    fn test_is_default_value() {
        assert!(is_default_value("svg", "version", "1.1"));
        assert!(is_default_value("rect", "opacity", "1"));
        assert!(!is_default_value("rect", "opacity", "0.5"));
    }
}
