//! HTML minification plugin for dodeca

use plugcard::{plugcard, PlugResult};

plugcard::export_plugin!();

/// Minify HTML content
///
/// Returns minified HTML, or an error if minification fails.
#[plugcard]
pub fn minify_html(html: String) -> PlugResult<String> {
    let cfg = minify_html::Cfg {
        minify_css: true,
        minify_js: true,
        // Preserve template syntax for compatibility
        preserve_brace_template_syntax: true,
        ..minify_html::Cfg::default()
    };

    let result = minify_html::minify(html.as_bytes(), &cfg);
    match String::from_utf8(result) {
        Ok(minified) => PlugResult::Ok(minified),
        Err(_) => PlugResult::Err("minification produced invalid UTF-8".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minify_html() {
        let input = r#"<!DOCTYPE html>
<html>
  <head>
    <title>Test</title>
  </head>
  <body>
    <p>Hello World</p>
  </body>
</html>"#;

        let result = minify_html(input.to_string());
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(output.len() < input.len());
        assert!(output.contains("<p>Hello World"));
        assert!(output.contains("<title>Test"));
    }

    #[test]
    fn test_minify_with_css() {
        let input = r#"<style>
  body {
    color: red;
  }
</style>"#;

        let result = minify_html(input.to_string());
        let PlugResult::Ok(output) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(output.len() < input.len());
        assert!(output.contains("color:red") || output.contains("color: red"));
    }
}
