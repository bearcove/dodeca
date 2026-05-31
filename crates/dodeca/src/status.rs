//! Server-rendered HTML for the running-server status page.
//!
//! This module produces **only a string** — no HTTP. The serving is done by the
//! http cell (HTTP lives in the cell, not here); [`SiteServer::status_html`]
//! gathers the live data and calls [`render_status_html`], and the
//! `ContentService` hands the result back to the cell to serve.

use crate::config::ResolvedSource;

/// Live inputs for the status page, gathered from the running server.
pub struct StatusData<'a> {
    pub sources: &'a [ResolvedSource],
    /// All loaded source registry keys (mount-prefixed), for per-source counts.
    pub source_keys: Vec<String>,
    pub generation: u64,
    pub error_routes: Vec<String>,
    pub uptime_secs: u64,
    /// The public content port (for showing the pull-webhook URL).
    pub site_port: u16,
}

struct SourceRow {
    name: String,
    mount: String,
    kind: &'static str,
    location: String,
    present: bool,
    pages: usize,
}

fn source_rows(data: &StatusData) -> Vec<SourceRow> {
    let mut pages: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for key in &data.source_keys {
        if let Some((src, _)) = crate::build_context::source_for_key(data.sources, key) {
            *pages.entry(src.name.clone()).or_default() += 1;
        }
    }
    data.sources
        .iter()
        .map(|s| {
            let (kind, location, present) = match &s.checkout_dir {
                Some(checkout) => ("git", checkout.to_string(), checkout.exists()),
                None => ("local", s.content_dir.to_string(), s.content_dir.exists()),
            };
            SourceRow {
                name: if s.name.is_empty() {
                    "(root)".to_string()
                } else {
                    s.name.clone()
                },
                mount: s.mount.clone(),
                kind,
                location,
                present,
                pages: pages.get(&s.name).copied().unwrap_or(0),
            }
        })
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn humanize_uptime(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// Render the status page HTML. Pure: no HTTP, no I/O beyond `Path::exists`
/// checks for source presence. Auto-refreshes via `<meta refresh>`.
pub fn render_status_html(data: &StatusData) -> String {
    let rows = source_rows(data);
    let total_pages: usize = rows.iter().map(|r| r.pages).sum();
    let uptime = humanize_uptime(data.uptime_secs);

    let source_rows_html: String = rows
        .iter()
        .map(|r| {
            let present = if r.present {
                "<span class=ok>present</span>"
            } else {
                "<span class=bad>absent</span>"
            };
            format!(
                "<tr><td><code>{}</code></td><td><code>{}</code></td><td>{}</td><td>{}</td>\
                 <td class=num>{}</td><td><code>{}</code></td></tr>",
                esc(&r.name),
                esc(&r.mount),
                r.kind,
                present,
                r.pages,
                esc(&r.location),
            )
        })
        .collect();

    let errors_html = if data.error_routes.is_empty() {
        "<p class=ok>No render errors.</p>".to_string()
    } else {
        format!(
            "<p class=bad>{} route(s) with errors:</p><ul>{}</ul>",
            data.error_routes.len(),
            data.error_routes
                .iter()
                .map(|r| format!("<li><code>{}</code></li>", esc(r)))
                .collect::<String>()
        )
    };

    format!(
        "<!doctype html><html><head><meta charset=utf-8>\
<meta http-equiv=refresh content=3>\
<title>dodeca status</title>\
<style>\
body{{font:14px/1.5 system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;color:#1a1a1a}}\
h1{{font-size:1.3rem}}h2{{font-size:1rem;margin-top:1.6rem;color:#555}}\
table{{border-collapse:collapse;width:100%}}\
th,td{{text-align:left;padding:.35rem .6rem;border-bottom:1px solid #eee}}\
th{{color:#888;font-weight:600;font-size:.8rem}}\
td.num{{text-align:right;font-variant-numeric:tabular-nums}}\
code{{background:#f5f5f5;padding:0 .25rem;border-radius:3px}}\
.ok{{color:#0a0}}.bad{{color:#c00;font-weight:600}}.meta{{color:#888}}\
</style></head><body>\
<h1>dodeca status</h1>\
<p class=meta>revision <b>{}</b> · {total_pages} pages · uptime {uptime} · auto-refreshing</p>\
<h2>Sources</h2>\
<table><thead><tr><th>name</th><th>mount</th><th>kind</th><th>state</th>\
<th class=num>pages</th><th>location</th></tr></thead><tbody>{source_rows_html}</tbody></table>\
<h2>Render</h2>{errors_html}\
<h2>Triggers</h2>\
<p>Pull a git source: <code>GET /_dodeca/pull</code> (all) or \
<code>/_dodeca/pull/&lt;name&gt;</code> on the content port \
(<code>:{}</code>). Polling via <code>--git-poll</code>.</p>\
</body></html>",
        data.generation, data.site_port,
    )
}
