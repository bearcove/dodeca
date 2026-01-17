//! HTML renderer for terminal output
//!
//! Converts parsed screen state to HTML with <t-*> custom tags.

use crate::parser::{Color, Decoration, FontStyle, Screen, Style, Weight};

/// Render screen state to HTML with <t-*> tags
pub fn render(screen: &Screen) -> String {
    let mut output = String::new();
    let mut has_content = false;

    // Track open tags for proper nesting
    let mut current_style: Option<Style> = None;

    for line in &screen.lines {
        let line_is_whitespace = line.iter().all(|sc| sc.c.is_whitespace());
        if line_is_whitespace {
            if has_content {
                output.push('\n');
            }
            continue;
        }
        has_content = true;

        for c in line {
            // Skip if just whitespace at end
            if c.c == ' ' && c.style == Style::default() {
                output.push(' ');
                continue;
            }

            // Close previous tags if style changed
            if current_style.is_some() && current_style != Some(c.style) {
                close_tags(&mut output, current_style.unwrap());
                current_style = None;
            }

            // Open new tags if needed
            if c.style != Style::default() && current_style.is_none() {
                open_tags(&mut output, c.style);
                current_style = Some(c.style);
            }

            // Output the character (with HTML escaping)
            match c.c {
                '&' => output.push_str("&amp;"),
                '<' => output.push_str("&lt;"),
                '>' => output.push_str("&gt;"),
                '"' => output.push_str("&quot;"),
                '\'' => output.push_str("&#x27;"),
                _ => output.push(c.c),
            }
        }

        // Trim trailing spaces from line
        while output.ends_with(' ') {
            output.pop();
        }

        // Close any open tags at end of line
        if let Some(style) = current_style.take() {
            close_tags(&mut output, style);
        }

        output.push('\n');
    }

    // Trim trailing whitespace
    while output.ends_with('\n') || output.ends_with(' ') {
        output.pop();
    }

    output
}

/// Open tags for a style (order: weight, decoration, font_style, bg, fg)
fn open_tags(output: &mut String, style: Style) {
    // Weight
    match style.weight {
        Weight::Bold => output.push_str("<t-b>"),
        Weight::Faint => output.push_str("<t-l>"),
        Weight::Normal => {}
    }

    // Decoration
    match style.decoration {
        Decoration::Underline => output.push_str("<t-u>"),
        Decoration::Strikethrough => output.push_str("<t-st>"),
        Decoration::None => {}
    }

    // Font style
    if style.font_style == FontStyle::Italic {
        output.push_str("<t-i>");
    }

    // Background color
    match style.bg {
        Color::Default => {}
        Color::Classic(c) => {
            output.push_str("<t-b");
            output.push_str(c.tag_suffix());
            output.push('>');
        }
        Color::Ansi256(n) => {
            output.push_str("<t-b");
            output.push_str(&n.to_string());
            output.push('>');
        }
        Color::Rgb(r, g, b) => {
            output.push_str("<t-b style=\"--c:#");
            output.push_str(&format!("{r:02x}{g:02x}{b:02x}"));
            output.push_str("\">");
        }
    }

    // Foreground color
    match style.fg {
        Color::Default => {}
        Color::Classic(c) => {
            output.push_str("<t-f");
            output.push_str(c.tag_suffix());
            output.push('>');
        }
        Color::Ansi256(n) => {
            output.push_str("<t-f");
            output.push_str(&n.to_string());
            output.push('>');
        }
        Color::Rgb(r, g, b) => {
            output.push_str("<t-f style=\"--c:#");
            output.push_str(&format!("{r:02x}{g:02x}{b:02x}"));
            output.push_str("\">");
        }
    }
}

/// Close tags for a style (reverse order of open)
fn close_tags(output: &mut String, style: Style) {
    // Foreground color
    match style.fg {
        Color::Default => {}
        Color::Classic(c) => {
            output.push_str("</t-f");
            output.push_str(c.tag_suffix());
            output.push('>');
        }
        Color::Ansi256(n) => {
            output.push_str("</t-f");
            output.push_str(&n.to_string());
            output.push('>');
        }
        Color::Rgb(_, _, _) => {
            output.push_str("</t-f>");
        }
    }

    // Background color
    match style.bg {
        Color::Default => {}
        Color::Classic(c) => {
            output.push_str("</t-b");
            output.push_str(c.tag_suffix());
            output.push('>');
        }
        Color::Ansi256(n) => {
            output.push_str("</t-b");
            output.push_str(&n.to_string());
            output.push('>');
        }
        Color::Rgb(_, _, _) => {
            output.push_str("</t-b>");
        }
    }

    // Font style
    if style.font_style == FontStyle::Italic {
        output.push_str("</t-i>");
    }

    // Decoration
    match style.decoration {
        Decoration::Underline => output.push_str("</t-u>"),
        Decoration::Strikethrough => output.push_str("</t-st>"),
        Decoration::None => {}
    }

    // Weight
    match style.weight {
        Weight::Bold => output.push_str("</t-b>"),
        Weight::Faint => output.push_str("</t-l>"),
        Weight::Normal => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn test_render_bold_red() {
        let input = b"\x1b[1;31mHello\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert_eq!(html, "<t-b><t-fred>Hello</t-fred></t-b>");
    }

    #[test]
    fn test_render_underline_green() {
        let input = b"\x1b[4;32mWorld\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert_eq!(html, "<t-u><t-fgrn>World</t-fgrn></t-u>");
    }

    #[test]
    fn test_render_256_color() {
        let input = b"\x1b[38;5;42mTest\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert_eq!(html, "<t-f42>Test</t-f42>");
    }

    #[test]
    fn test_render_rgb_color() {
        let input = b"\x1b[38;2;255;128;0mOrange\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert_eq!(html, "<t-f style=\"--c:#ff8000\">Orange</t-f>");
    }
}
