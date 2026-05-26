use camino::{Utf8Path, Utf8PathBuf};

const DODECA_HTML_SUFFIX: &str = ".ddc.html";
const HTML_SUFFIX: &str = ".html";

pub fn logical_template_path(relative: &Utf8Path) -> Option<String> {
    let path = relative.as_str();
    if let Some(stem) = path.strip_suffix(DODECA_HTML_SUFFIX) {
        Some(format!("{stem}{HTML_SUFFIX}"))
    } else if path.ends_with(HTML_SUFFIX) {
        Some(path.to_string())
    } else {
        None
    }
}

pub fn physical_template_path(templates_dir: &Utf8Path, logical_path: &str) -> Utf8PathBuf {
    let exact = templates_dir.join(logical_path);
    if exact.exists() {
        return exact;
    }

    if let Some(stem) = logical_path.strip_suffix(HTML_SUFFIX) {
        let dodeca_html = templates_dir.join(format!("{stem}{DODECA_HTML_SUFFIX}"));
        if dodeca_html.exists() {
            return dodeca_html;
        }
    }

    exact
}
