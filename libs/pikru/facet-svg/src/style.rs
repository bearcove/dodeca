//! SVG style attribute parsing and structured representation.

use facet::Facet;

/// A color value
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    /// No color (transparent)
    None,
    /// RGB color
    Rgb { r: u8, g: u8, b: u8 },
    /// Named color
    Named(String),
}

impl Color {
    /// Parse a color from a string
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.eq_ignore_ascii_case("none") {
            return Color::None;
        }

        // Try parsing rgb(r,g,b)
        if let Some(inner) = s.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    parts[0].trim().parse::<u8>(),
                    parts[1].trim().parse::<u8>(),
                    parts[2].trim().parse::<u8>(),
                ) {
                    return Color::Rgb { r, g, b };
                }
            }
        }

        // Try parsing hex color
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    return Color::Rgb { r, g, b };
                }
            } else if hex.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..1], 16),
                    u8::from_str_radix(&hex[1..2], 16),
                    u8::from_str_radix(&hex[2..3], 16),
                ) {
                    // Expand 3-digit hex: #abc -> #aabbcc
                    return Color::Rgb {
                        r: r * 17,
                        g: g * 17,
                        b: b * 17,
                    };
                }
            }
        }

        // Named color - normalize common names to RGB
        match s.to_lowercase().as_str() {
            "black" => Color::Rgb { r: 0, g: 0, b: 0 },
            "white" => Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            "red" => Color::Rgb { r: 255, g: 0, b: 0 },
            "green" => Color::Rgb { r: 0, g: 128, b: 0 },
            "blue" => Color::Rgb { r: 0, g: 0, b: 255 },
            "yellow" => Color::Rgb {
                r: 255,
                g: 255,
                b: 0,
            },
            "cyan" => Color::Rgb {
                r: 0,
                g: 255,
                b: 255,
            },
            "magenta" => Color::Rgb {
                r: 255,
                g: 0,
                b: 255,
            },
            "gray" | "grey" => Color::Rgb {
                r: 128,
                g: 128,
                b: 128,
            },
            _ => Color::Named(s.to_string()),
        }
    }

    /// Serialize to string in rgb() format like C pikchr
    pub fn to_string(&self) -> String {
        match self {
            Color::None => "none".to_string(),
            Color::Rgb { r, g, b } => format!("rgb({},{},{})", r, g, b),
            Color::Named(n) => n.clone(),
        }
    }
}

/// Structured SVG style attribute
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SvgStyle {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    pub stroke_width: Option<f64>,
    pub stroke_dasharray: Option<(f64, f64)>,
}

impl SvgStyle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse style from a CSS-like string (e.g., "fill:none;stroke-width:2.16;stroke:rgb(0,0,0);")
    pub fn parse(s: &str) -> Result<Self, StyleParseError> {
        let mut style = SvgStyle::new();

        for part in s.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (key, value) = part
                .split_once(':')
                .ok_or_else(|| StyleParseError::InvalidProperty(part.to_string()))?;

            let key = key.trim();
            let value = value.trim();

            match key {
                "fill" => {
                    style.fill = Some(Color::parse(value));
                }
                "stroke" => {
                    style.stroke = Some(Color::parse(value));
                }
                "stroke-width" => {
                    style.stroke_width = Some(
                        value
                            .parse()
                            .map_err(|_| StyleParseError::InvalidNumber(value.to_string()))?,
                    );
                }
                "stroke-dasharray" => {
                    let parts: Vec<&str> = value.split(',').collect();
                    if parts.len() >= 2 {
                        let a = parts[0]
                            .trim()
                            .parse()
                            .map_err(|_| StyleParseError::InvalidNumber(parts[0].to_string()))?;
                        let b = parts[1]
                            .trim()
                            .parse()
                            .map_err(|_| StyleParseError::InvalidNumber(parts[1].to_string()))?;
                        style.stroke_dasharray = Some((a, b));
                    }
                }
                _ => {
                    // Ignore unknown properties
                }
            }
        }

        Ok(style)
    }

    /// Serialize to CSS-like string with trailing semicolon (like C pikchr)
    pub fn to_string(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref fill) = self.fill {
            parts.push(format!("fill:{}", fill.to_string()));
        }
        if let Some(stroke_width) = self.stroke_width {
            parts.push(format!("stroke-width:{}", fmt_num(stroke_width)));
        }
        if let Some(ref stroke) = self.stroke {
            parts.push(format!("stroke:{}", stroke.to_string()));
        }
        if let Some((a, b)) = self.stroke_dasharray {
            parts.push(format!("stroke-dasharray:{},{}", fmt_num(a), fmt_num(b)));
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("{};", parts.join(";"))
        }
    }
}

/// Format a number like C pikchr's %.10g
fn fmt_num(v: f64) -> String {
    let s = format!("{:.10}", v);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

/// Error parsing style
#[derive(Debug, Clone, PartialEq)]
pub enum StyleParseError {
    InvalidProperty(String),
    InvalidNumber(String),
}

impl std::fmt::Display for StyleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StyleParseError::InvalidProperty(s) => write!(f, "invalid style property: {}", s),
            StyleParseError::InvalidNumber(s) => write!(f, "invalid number in style: {}", s),
        }
    }
}

impl std::error::Error for StyleParseError {}

/// Proxy type for SvgStyle - serializes as a string
#[derive(Facet, Clone, Debug)]
#[facet(transparent)]
pub struct SvgStyleProxy(pub String);

impl TryFrom<SvgStyleProxy> for SvgStyle {
    type Error = StyleParseError;
    fn try_from(proxy: SvgStyleProxy) -> Result<Self, Self::Error> {
        SvgStyle::parse(&proxy.0)
    }
}

impl TryFrom<&SvgStyle> for SvgStyleProxy {
    type Error = std::convert::Infallible;
    fn try_from(v: &SvgStyle) -> Result<Self, Self::Error> {
        Ok(SvgStyleProxy(v.to_string()))
    }
}

// Option impls for facet proxy support
impl From<SvgStyleProxy> for Option<SvgStyle> {
    fn from(proxy: SvgStyleProxy) -> Self {
        SvgStyle::parse(&proxy.0).ok()
    }
}

impl TryFrom<&Option<SvgStyle>> for SvgStyleProxy {
    type Error = std::convert::Infallible;
    fn try_from(v: &Option<SvgStyle>) -> Result<Self, Self::Error> {
        match v {
            Some(style) => Ok(SvgStyleProxy(style.to_string())),
            None => Ok(SvgStyleProxy(String::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_rgb() {
        assert_eq!(Color::parse("rgb(0,0,0)"), Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(
            Color::parse("rgb(255,128,64)"),
            Color::Rgb {
                r: 255,
                g: 128,
                b: 64
            }
        );
    }

    #[test]
    fn test_parse_color_named() {
        assert_eq!(Color::parse("black"), Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(Color::parse("none"), Color::None);
    }

    #[test]
    fn test_parse_style() {
        let style = SvgStyle::parse("fill:none;stroke-width:2.16;stroke:rgb(0,0,0);").unwrap();
        assert_eq!(style.fill, Some(Color::None));
        assert_eq!(style.stroke_width, Some(2.16));
        assert_eq!(style.stroke, Some(Color::Rgb { r: 0, g: 0, b: 0 }));
    }

    #[test]
    fn test_style_roundtrip() {
        let original = "fill:none;stroke-width:2.16;stroke:rgb(0,0,0);";
        let style = SvgStyle::parse(original).unwrap();
        let serialized = style.to_string();
        let reparsed = SvgStyle::parse(&serialized).unwrap();
        assert_eq!(style, reparsed);
    }

    #[test]
    fn test_color_normalization() {
        // "black" and "rgb(0,0,0)" should compare equal
        let c1 = Color::parse("black");
        let c2 = Color::parse("rgb(0,0,0)");
        assert_eq!(c1, c2);
    }
}
