//! ANSI to HTML conversion
//!
//! Converts ANSI escape codes to HTML spans with inline styles.

/// Convert ANSI escape codes to HTML spans with inline styles.
///
/// Supports:
/// - Basic styles: bold, dim, italic, underline
/// - Standard colors (30-37, 90-97)
/// - 24-bit RGB colors (38;2;r;g;b)
pub fn ansi_to_html(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    let mut in_span = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['

            // Parse the escape sequence
            let mut seq = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_ascii_digit() || ch == ';' {
                    seq.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Consume the final character (usually 'm')
            let final_char = chars.next();

            if final_char == Some('m') {
                // Close any existing span
                if in_span {
                    output.push_str("</span>");
                    in_span = false;
                }

                // Parse the style
                if let Some(style) = parse_ansi_style(&seq)
                    && !style.is_empty()
                {
                    output.push_str(&format!("<span style=\"{style}\">"));
                    in_span = true;
                }
            }
        } else if c == '<' {
            output.push_str("&lt;");
        } else if c == '>' {
            output.push_str("&gt;");
        } else if c == '&' {
            output.push_str("&amp;");
        } else {
            output.push(c);
        }
    }

    if in_span {
        output.push_str("</span>");
    }

    output
}

/// Parse ANSI style codes and return CSS style string.
fn parse_ansi_style(seq: &str) -> Option<String> {
    if seq.is_empty() || seq == "0" {
        return Some(String::new()); // Reset
    }

    let parts: Vec<&str> = seq.split(';').collect();
    let mut styles = Vec::new();

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "0" => return Some(String::new()), // Reset
            "1" => styles.push("font-weight:bold".to_string()),
            "2" => styles.push("opacity:0.7".to_string()), // Dim
            "3" => styles.push("font-style:italic".to_string()),
            "4" => styles.push("text-decoration:underline".to_string()),
            "30" => styles.push("color:#000".to_string()),
            "31" => styles.push("color:#e06c75".to_string()), // Red
            "32" => styles.push("color:#98c379".to_string()), // Green
            "33" => styles.push("color:#e5c07b".to_string()), // Yellow
            "34" => styles.push("color:#61afef".to_string()), // Blue
            "35" => styles.push("color:#c678dd".to_string()), // Magenta
            "36" => styles.push("color:#56b6c2".to_string()), // Cyan
            "37" => styles.push("color:#abb2bf".to_string()), // White
            "38" => {
                // Extended color (24-bit RGB)
                if i + 1 < parts.len() && parts[i + 1] == "2" && i + 4 < parts.len() {
                    let r = parts[i + 2];
                    let g = parts[i + 3];
                    let b = parts[i + 4];
                    styles.push(format!("color:rgb({r},{g},{b})"));
                    i += 4;
                }
            }
            "90" => styles.push("color:#5c6370".to_string()), // Bright black (gray)
            "91" => styles.push("color:#e06c75".to_string()), // Bright red
            "92" => styles.push("color:#98c379".to_string()), // Bright green
            "93" => styles.push("color:#e5c07b".to_string()), // Bright yellow
            "94" => styles.push("color:#61afef".to_string()), // Bright blue
            "95" => styles.push("color:#c678dd".to_string()), // Bright magenta
            "96" => styles.push("color:#56b6c2".to_string()), // Bright cyan
            "97" => styles.push("color:#fff".to_string()),    // Bright white
            _ => {}
        }
        i += 1;
    }

    Some(styles.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ansi_to_html_plain_text() {
        assert_eq!(ansi_to_html("hello world"), "hello world");
    }

    #[test]
    fn test_ansi_to_html_escapes_html() {
        assert_eq!(ansi_to_html("<script>"), "&lt;script&gt;");
        assert_eq!(ansi_to_html("a & b"), "a &amp; b");
    }

    #[test]
    fn test_ansi_to_html_bold() {
        let input = "\x1b[1mbold\x1b[0m normal";
        let output = ansi_to_html(input);
        assert!(output.contains("font-weight:bold"));
        assert!(output.contains("bold</span>"));
    }

    #[test]
    fn test_ansi_to_html_colors() {
        let input = "\x1b[31mred\x1b[0m";
        let output = ansi_to_html(input);
        assert!(output.contains("color:#e06c75"));
    }
}
