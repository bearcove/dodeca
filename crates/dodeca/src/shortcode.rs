//! Shortcode resolution: replace `<dodeca-shortcode>` placeholders with gingembre-rendered HTML.
//!
//! cell-markdown emits placeholder elements during markdown rendering so shortcode
//! templates can be read through the picante-tracked template path. This module
//! performs the substitution inside the picante render query, inheriting dependency
//! tracking for free.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use cell_markdown_proto::ShortcodeArgsProto;
use facet_value::{DestructuredRef, VObject, VString, Value};

use crate::cells::render_template as render_template_direct;
use crate::db::SiteTree;
use crate::template_host::{RenderContext, RenderContextGuard};

/// Replace all `<dodeca-shortcode>` placeholder elements in `html` with their
/// rendered template output.
///
/// Processes elements innermost-first: finds the first `</dodeca-shortcode>`,
/// looks backward for its matching opening tag, renders it, then repeats.
/// This naturally handles nested shortcodes (e.g., a shortcode body that itself
/// contains shortcodes).
pub async fn resolve_shortcodes(
    mut html: String,
    templates: &HashMap<String, String>,
    site_tree: &SiteTree,
) -> String {
    const OPEN: &str = "<dodeca-shortcode ";
    const CLOSE: &str = "</dodeca-shortcode>";

    // The first closing tag is always the innermost shortcode; resolve until none remain.
    while let Some(close_pos) = html.find(CLOSE) {
        // Find the matching opening tag by scanning backward from close_pos.
        let prefix = &html[..close_pos];
        let Some(open_rel) = prefix.rfind(OPEN) else {
            tracing::warn!("dodeca-shortcode closing tag without matching opening tag");
            break;
        };

        let open_pos = open_rel;
        let attrs_start = open_pos + OPEN.len();

        // Find the `>` that ends the opening tag.
        let Some(gt_rel) = html[attrs_start..].find('>') else {
            tracing::warn!("dodeca-shortcode opening tag not terminated");
            break;
        };
        let gt_pos = attrs_start + gt_rel;

        let attrs = &html[attrs_start..gt_pos];
        let body_start = gt_pos + 1;
        let body_end = close_pos;
        let close_end = close_pos + CLOSE.len();

        let name = parse_attr(attrs, "data-name").unwrap_or_default();
        let args_b64 = parse_attr(attrs, "data-args").unwrap_or_default();
        let body = html[body_start..body_end].to_string();

        tracing::debug!(name, "resolving shortcode");

        let rendered = render_one_shortcode(&name, &args_b64, &body, templates, site_tree).await;
        html.replace_range(open_pos..close_end, &rendered);
    }

    html
}

/// Extract the value of an HTML attribute from an attributes string.
///
/// Handles `key="value"` and `key='value'` forms. Returns `None` if not found.
fn parse_attr(attrs: &str, key: &str) -> Option<String> {
    let search = format!("{key}=\"");
    let start = attrs.find(&search)? + search.len();
    let end = attrs[start..].find('"')? + start;
    // HTML-unescape the common entities that we might have encoded
    Some(
        attrs[start..end]
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#x27;", "'"),
    )
}

/// Render a single shortcode invocation through gingembre.
async fn render_one_shortcode(
    name: &str,
    args_b64: &str,
    body: &str,
    templates: &HashMap<String, String>,
    site_tree: &SiteTree,
) -> String {
    let args_proto = decode_args(args_b64);

    // `include` is a built-in shortcode: pull another file's content into the
    // page (e.g. a crate's README), optionally cleaned up for embedding via
    // `strip_readme`. Resolved here rather than as a gingembre template because
    // it reads a file and renders markdown.
    if name == "include" {
        return render_include(args_proto.as_ref()).await;
    }

    let template_name = format!("shortcodes/{name}.html");

    // Check the template exists before attempting render.
    if !templates.contains_key(&template_name) {
        tracing::warn!(
            name,
            template_name,
            "shortcode template not found; leaving placeholder"
        );
        return format!("<!-- shortcode '{name}' template not found -->");
    }

    let db = match crate::db::TASK_DB.try_with(|db| db.clone()) {
        Ok(db) => db,
        Err(_) => {
            tracing::warn!(name, "TASK_DB not set; cannot render shortcode");
            return format!("<!-- shortcode '{name}': no db context -->");
        }
    };

    // The page route isn't threaded into shortcode rendering, so a source-scoped
    // host function (`build()`) called from inside a shortcode resolves to the
    // root source (`/`). Threading the owning route here is a follow-up.
    let context = RenderContext::new(
        templates.clone(),
        db,
        Arc::new(site_tree.clone()),
        "/".to_string(),
    );
    let guard = RenderContextGuard::new(context);

    let initial_context = build_shortcode_context(args_proto.as_ref(), body);

    match render_template_direct(guard.id(), &template_name, initial_context).await {
        Ok(cell_gingembre_proto::RenderResult::Success { html }) => html,
        Ok(cell_gingembre_proto::RenderResult::Error { error }) => {
            tracing::warn!(
                name,
                message = error.message,
                "shortcode template render error"
            );
            format!(
                "<!-- shortcode '{name}' render error: {} -->",
                error.message
            )
        }
        Err(e) => {
            tracing::warn!(name, error = %e, "shortcode gingembre error");
            format!("<!-- shortcode '{name}' error: {e} -->")
        }
    }
}

/// Render the built-in `include` shortcode: read a file relative to the project
/// root, optionally run the `strip_readme` cleanup, and render it as markdown.
///
/// `{{ include(path="figue/README.md", strip_readme=true) }}`
///
/// Note: this reads the file directly for now; making the included file a
/// tracked picante input (so editing it hot-reloads dependents) is the next step.
async fn render_include(args: Option<&ShortcodeArgsProto>) -> String {
    let Some(path) = include_arg(args, "path") else {
        tracing::warn!("include: missing `path` argument");
        return "<!-- include: missing `path` -->".to_string();
    };
    let strip = include_arg(args, "strip_readme").as_deref() == Some("true");

    let Some(cfg) = crate::config::global_config() else {
        return "<!-- include: config not initialized -->".to_string();
    };

    // Read through the tracked include registry (falls back to a direct read,
    // and records a dependency so an edited file hot-reloads this page).
    let Some(raw) = crate::includes::read(&path, &cfg._root) else {
        tracing::warn!(path = %path, root = %cfg._root, "include: failed to read file");
        return format!("<!-- include: cannot read '{path}' -->");
    };

    let content = if strip {
        strip_readme_markdown(&raw)
    } else {
        raw
    };

    match crate::cells::parse_and_render_markdown(&path, &content, false, false).await {
        Ok(cell_markdown_proto::ParseResult::Success { html, .. }) => html,
        other => {
            tracing::warn!(path = %path, ?other, "include: markdown render failed");
            format!("<!-- include: render failed for '{path}' -->")
        }
    }
}

/// Look up a single named argument from a shortcode's `Pairs` args.
fn include_arg(args: Option<&ShortcodeArgsProto>, key: &str) -> Option<String> {
    match args? {
        ShortcodeArgsProto::Pairs(pairs) => {
            pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        }
        ShortcodeArgsProto::Yaml(_) => None,
    }
}

/// Clean up a crate README for embedding in a docs page: drop the leading `#`
/// title and the run of shield/badge lines, and honor rustdoc's `# `-prefixed
/// hidden lines inside ```rust code blocks (so `# fn main()` scaffolding doesn't
/// show, and `##` unescapes to `#`).
fn strip_readme_markdown(md: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    // `Some(is_rust)` while inside a fenced code block.
    let mut fence: Option<bool> = None;
    // Still consuming the leading title/badge preamble.
    let mut in_preamble = true;

    for line in md.lines() {
        let t = line.trim_start();

        if t.starts_with("```") || t.starts_with("~~~") {
            if fence.is_none() {
                let lang = t.trim_start_matches(['`', '~']).trim();
                let is_rust =
                    lang.is_empty() || lang == "rust" || lang == "rs" || lang.starts_with("rust,");
                fence = Some(is_rust);
            } else {
                fence = None;
            }
            in_preamble = false;
            out.push(line.to_string());
            continue;
        }

        if let Some(is_rust) = fence {
            if is_rust {
                let ct = line.trim_start();
                if ct == "#" || ct.starts_with("# ") {
                    continue; // rustdoc hidden line
                }
                if let Some(rest) = ct.strip_prefix("##") {
                    let indent = &line[..line.len() - ct.len()];
                    out.push(format!("{indent}#{rest}")); // `##` -> `#`
                    continue;
                }
            }
            out.push(line.to_string());
            continue;
        }

        if in_preamble {
            if t.is_empty() {
                continue;
            }
            if t.starts_with("# ") {
                continue; // the title
            }
            if is_badge_line(t) {
                continue;
            }
            in_preamble = false;
        }
        out.push(line.to_string());
    }

    let joined = out.join("\n");
    format!("{}\n", joined.trim_start_matches('\n').trim_end())
}

/// A markdown line that's just a shield/badge image-link (or a row of them).
fn is_badge_line(t: &str) -> bool {
    t.contains("shields.io") || (t.starts_with("[![") && t.ends_with(')'))
}

/// Decode base64 + facet-json args from the `data-args` attribute.
fn decode_args(args_b64: &str) -> Option<ShortcodeArgsProto> {
    let json_bytes = base64::engine::general_purpose::STANDARD
        .decode(args_b64)
        .ok()?;
    let json_str = std::str::from_utf8(&json_bytes).ok()?;
    facet_json::from_str::<ShortcodeArgsProto>(json_str).ok()
}

/// Build the initial gingembre context Value from the shortcode args and body.
///
/// For `Pairs` args: each key=value pair becomes a top-level variable.
/// For `Yaml` args: parse the YAML, find the mapping under the shortcode name key,
///   and spread its entries as top-level variables.
/// Body (pre-rendered HTML) is always set as `body` (marked safe).
fn build_shortcode_context(args: Option<&ShortcodeArgsProto>, body: &str) -> Value {
    let mut obj = VObject::new();

    if let Some(args) = args {
        match args {
            ShortcodeArgsProto::Pairs(pairs) => {
                for (k, v) in pairs {
                    obj.insert(VString::from(k.as_str()), Value::from(v.as_str()));
                }
            }
            ShortcodeArgsProto::Yaml(yaml_text) => {
                if let Ok(doc) = facet_yaml::from_str::<Value>(yaml_text) {
                    // The YAML is a single-key mapping: `:name:\n  key: val\n ...`
                    // We find any mapping value at the top level and spread it.
                    if let DestructuredRef::Object(top) = doc.destructure_ref() {
                        // Take the first value that is itself a mapping and spread it.
                        for (_, val) in top.iter() {
                            if let DestructuredRef::Object(inner) = val.destructure_ref() {
                                for (k, v) in inner.iter() {
                                    obj.insert(VString::from(k.as_str()), v.clone());
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Body is pre-rendered HTML; mark it safe so gingembre doesn't re-escape it.
    let safe_body = VString::from(body).into_safe().into_value();
    obj.insert(VString::from("body"), safe_body);

    Value::from(obj)
}
