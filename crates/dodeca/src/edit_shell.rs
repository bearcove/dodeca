//! The in-browser editor shell — a pure HTML string (no HTTP here), served at
//! `GET /_dodeca/edit/<page>` to verified editors. It boots the vite/CodeMirror
//! editor app, handing it the page route and the editor session token via
//! root-element `data-` attributes (no inline JSON). The token authorizes the
//! `edit_*` vox RPC calls the app makes over the devtools websocket.

/// Render the editor shell for `route`, embedding the session `token`. `version`
/// cache-busts the (unhashed) entry asset URLs.
pub fn render_edit_shell(route: &str, token: &str, version: &str) -> String {
    let route = escape_attr(route);
    let token = escape_attr(token);
    let version = escape_attr(version);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>edit · {route}</title>
<link rel="stylesheet" href="/_/edit/edit.css?v={version}">
</head>
<body>
<div id="vixen-editor" data-route="{route}" data-token="{token}"></div>
<script type="module" src="/_/edit/edit.js?v={version}"></script>
</body>
</html>
"#
    )
}

/// Escape a string for safe inclusion in an HTML attribute (and text).
fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_route_and_token_in_data_attrs() {
        let html = render_edit_shell("/overview", "abc123", "v1");
        assert!(html.contains(r#"data-route="/overview""#));
        assert!(html.contains(r#"data-token="abc123""#));
    }

    #[test]
    fn escapes_attribute_injection() {
        let html = render_edit_shell("/\"><script>x", "t", "v1");
        assert!(!html.contains("<script>x"));
        assert!(html.contains("&quot;&gt;&lt;script&gt;x"));
    }
}
