use camino::{Utf8Path, Utf8PathBuf};

const DODECA_HTML_SUFFIX: &str = ".ddc.html";
const HTML_SUFFIX: &str = ".html";
const JINJA_SUFFIX: &str = ".jinja";

pub fn logical_template_path(relative: &Utf8Path) -> Option<String> {
    let path = relative.as_str();
    if let Some(stem) = path.strip_suffix(DODECA_HTML_SUFFIX) {
        Some(format!("{stem}{HTML_SUFFIX}"))
    } else if path.ends_with(HTML_SUFFIX) {
        Some(path.to_string())
    } else if let Some(without_jinja) = path.strip_suffix(JINJA_SUFFIX) {
        // foo.html.jinja → foo.html, and likewise foo.css.jinja → foo.css,
        // foo.xml.jinja → foo.xml: any templated file is addressable by its
        // un-suffixed name (e.g. `{% include "critical-styles.css" %}`).
        if without_jinja.is_empty() {
            None
        } else {
            Some(without_jinja.to_string())
        }
    } else {
        // Anything else under templates/ (an SVG icon, a text snippet) is
        // addressable verbatim, e.g. `{% include "icons/discord.svg" %}`.
        Some(path.to_string())
    }
}

pub fn physical_template_path(templates_dir: &Utf8Path, logical_path: &str) -> Utf8PathBuf {
    let exact = templates_dir.join(logical_path);
    if exact.exists() {
        return exact;
    }

    // `critical-styles.css` → `critical-styles.css.jinja`, for any extension.
    let jinja = templates_dir.join(format!("{logical_path}{JINJA_SUFFIX}"));
    if jinja.exists() {
        return jinja;
    }

    if let Some(stem) = logical_path.strip_suffix(HTML_SUFFIX) {
        let dodeca_html = templates_dir.join(format!("{stem}{DODECA_HTML_SUFFIX}"));
        if dodeca_html.exists() {
            return dodeca_html;
        }
        let jinja_html = templates_dir.join(format!("{stem}{HTML_SUFFIX}{JINJA_SUFFIX}"));
        if jinja_html.exists() {
            return jinja_html;
        }
    }

    exact
}
