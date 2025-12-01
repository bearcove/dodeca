//! SVG path data parsing and minification.
//!
//! SVG path syntax: https://www.w3.org/TR/SVG/paths.html

use crate::error::SavageError;

/// A parsed SVG path.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub commands: Vec<Command>,
}

/// A path command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// M/m - Move to
    MoveTo { rel: bool, x: f64, y: f64 },
    /// L/l - Line to
    LineTo { rel: bool, x: f64, y: f64 },
    /// H/h - Horizontal line to
    HorizontalTo { rel: bool, x: f64 },
    /// V/v - Vertical line to
    VerticalTo { rel: bool, y: f64 },
    /// C/c - Cubic bezier
    CurveTo {
        rel: bool,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        x: f64,
        y: f64,
    },
    /// S/s - Smooth cubic bezier
    SmoothCurveTo {
        rel: bool,
        x2: f64,
        y2: f64,
        x: f64,
        y: f64,
    },
    /// Q/q - Quadratic bezier
    QuadTo {
        rel: bool,
        x1: f64,
        y1: f64,
        x: f64,
        y: f64,
    },
    /// T/t - Smooth quadratic bezier
    SmoothQuadTo { rel: bool, x: f64, y: f64 },
    /// A/a - Arc
    Arc {
        rel: bool,
        rx: f64,
        ry: f64,
        x_axis_rotation: f64,
        large_arc: bool,
        sweep: bool,
        x: f64,
        y: f64,
    },
    /// Z/z - Close path
    ClosePath,
}

/// Parse SVG path data.
pub fn parse_path(d: &str) -> Result<Path, SavageError> {
    let mut parser = PathParser::new(d);
    parser.parse()
}

/// Serialize path data with the given precision.
pub fn serialize_path(path: &Path, precision: u8) -> String {
    let mut out = String::new();
    let mut prev_cmd: Option<char> = None;

    for cmd in &path.commands {
        let (c, new_cmd) = match cmd {
            Command::MoveTo { rel, x, y } => {
                let c = if *rel { 'm' } else { 'M' };
                (format_cmd(c, prev_cmd, &[*x, *y], precision), c)
            }
            Command::LineTo { rel, x, y } => {
                let c = if *rel { 'l' } else { 'L' };
                (format_cmd(c, prev_cmd, &[*x, *y], precision), c)
            }
            Command::HorizontalTo { rel, x } => {
                let c = if *rel { 'h' } else { 'H' };
                (format_cmd(c, prev_cmd, &[*x], precision), c)
            }
            Command::VerticalTo { rel, y } => {
                let c = if *rel { 'v' } else { 'V' };
                (format_cmd(c, prev_cmd, &[*y], precision), c)
            }
            Command::CurveTo {
                rel,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let c = if *rel { 'c' } else { 'C' };
                (
                    format_cmd(c, prev_cmd, &[*x1, *y1, *x2, *y2, *x, *y], precision),
                    c,
                )
            }
            Command::SmoothCurveTo { rel, x2, y2, x, y } => {
                let c = if *rel { 's' } else { 'S' };
                (format_cmd(c, prev_cmd, &[*x2, *y2, *x, *y], precision), c)
            }
            Command::QuadTo { rel, x1, y1, x, y } => {
                let c = if *rel { 'q' } else { 'Q' };
                (format_cmd(c, prev_cmd, &[*x1, *y1, *x, *y], precision), c)
            }
            Command::SmoothQuadTo { rel, x, y } => {
                let c = if *rel { 't' } else { 'T' };
                (format_cmd(c, prev_cmd, &[*x, *y], precision), c)
            }
            Command::Arc {
                rel,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => {
                let c = if *rel { 'a' } else { 'A' };
                let arc_str = format_arc(
                    c,
                    prev_cmd,
                    *rx,
                    *ry,
                    *x_axis_rotation,
                    *large_arc,
                    *sweep,
                    *x,
                    *y,
                    precision,
                );
                (arc_str, c)
            }
            Command::ClosePath => {
                let c = 'z';
                (format_cmd(c, prev_cmd, &[], precision), c)
            }
        };
        // Check if we need a separator between previous output and new command
        if !out.is_empty() && !c.is_empty() {
            let last = out.chars().last().unwrap();
            let first = c.chars().next().unwrap();
            if (last.is_ascii_digit() || last == '.') && (first.is_ascii_digit() || first == '.') {
                out.push(' ');
            }
        }
        out.push_str(&c);
        prev_cmd = Some(new_cmd);
    }

    out
}

fn format_cmd(cmd: char, prev_cmd: Option<char>, args: &[f64], precision: u8) -> String {
    let mut out = String::new();

    // Omit command letter if it's the same as previous (except for M which becomes L)
    let needs_cmd = match prev_cmd {
        None => true,
        Some(prev) => {
            // After M, coordinates are treated as L; after m, as l
            if prev == 'M' && cmd == 'L' {
                false
            } else if prev == 'm' && cmd == 'l' {
                false
            } else {
                prev != cmd
            }
        }
    };

    if needs_cmd && !args.is_empty() {
        out.push(cmd);
    } else if args.is_empty() {
        // Always write z/Z
        out.push(cmd);
    }

    for arg in args.iter() {
        let formatted = format_number(*arg, precision);
        // Add separator if needed
        let needs_sep = if out.is_empty() {
            false
        } else {
            let last = out.chars().last().unwrap();
            let first = formatted.chars().next().unwrap();
            // Need space if both are digits or if previous is digit and current starts with .
            (last.is_ascii_digit() || last == '.') && (first.is_ascii_digit() || first == '.')
        };
        if needs_sep {
            out.push(' ');
        }
        out.push_str(&formatted);
    }

    out
}

fn format_arc(
    cmd: char,
    prev_cmd: Option<char>,
    rx: f64,
    ry: f64,
    x_rot: f64,
    large_arc: bool,
    sweep: bool,
    x: f64,
    y: f64,
    precision: u8,
) -> String {
    let mut out = String::new();

    if prev_cmd != Some(cmd) {
        out.push(cmd);
    }

    let parts = [
        format_number(rx, precision),
        format_number(ry, precision),
        format_number(x_rot, precision),
        if large_arc { "1".into() } else { "0".into() },
        if sweep { "1".into() } else { "0".into() },
        format_number(x, precision),
        format_number(y, precision),
    ];

    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            let last = out.chars().last().unwrap();
            let first = part.chars().next().unwrap();
            if last.is_ascii_digit() && (first.is_ascii_digit() || first == '.') {
                out.push(' ');
            }
        }
        out.push_str(part);
    }

    out
}

/// Format a number with the given precision, removing unnecessary zeros.
pub fn format_number(n: f64, precision: u8) -> String {
    if n == 0.0 {
        return "0".into();
    }

    // Round to precision
    let factor = 10f64.powi(precision as i32);
    let rounded = (n * factor).round() / factor;

    // Check if it's an integer
    if rounded.fract() == 0.0 {
        return format!("{}", rounded as i64);
    }

    // Format with precision then trim trailing zeros
    let mut s = format!("{:.prec$}", rounded, prec = precision as usize);

    // Trim trailing zeros after decimal point
    if s.contains('.') {
        s = s.trim_end_matches('0').to_string();
        s = s.trim_end_matches('.').to_string();
    }

    // Remove leading zero before decimal: 0.5 -> .5
    if s.starts_with("0.") {
        s = s[1..].to_string();
    } else if s.starts_with("-0.") {
        s = format!("-{}", &s[2..]);
    }

    s
}

struct PathParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> PathParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(&mut self) -> Result<Path, SavageError> {
        let mut commands = Vec::new();
        let mut last_cmd: Option<char> = None;

        self.skip_whitespace();

        while !self.is_eof() {
            let cmd = if self.peek().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
                let c = self.next().unwrap();
                last_cmd = Some(c);
                c
            } else {
                // Implicit command - repeat last command
                // After M, implicit command is L; after m, it's l
                match last_cmd {
                    Some('M') => 'L',
                    Some('m') => 'l',
                    Some(c) => c,
                    None => {
                        return Err(SavageError::InvalidPath(
                            "Expected command letter".into(),
                        ))
                    }
                }
            };

            let parsed = self.parse_command(cmd)?;
            commands.push(parsed);
            self.skip_whitespace_and_comma();
        }

        Ok(Path { commands })
    }

    fn parse_command(&mut self, cmd: char) -> Result<Command, SavageError> {
        let rel = cmd.is_ascii_lowercase();

        match cmd.to_ascii_lowercase() {
            'm' => {
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::MoveTo { rel, x, y })
            }
            'l' => {
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::LineTo { rel, x, y })
            }
            'h' => {
                let x = self.parse_number()?;
                Ok(Command::HorizontalTo { rel, x })
            }
            'v' => {
                let y = self.parse_number()?;
                Ok(Command::VerticalTo { rel, y })
            }
            'c' => {
                let x1 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y1 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let x2 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y2 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::CurveTo {
                    rel,
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                })
            }
            's' => {
                let x2 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y2 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::SmoothCurveTo { rel, x2, y2, x, y })
            }
            'q' => {
                let x1 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y1 = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::QuadTo { rel, x1, y1, x, y })
            }
            't' => {
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::SmoothQuadTo { rel, x, y })
            }
            'a' => {
                let rx = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let ry = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let x_axis_rotation = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let large_arc = self.parse_flag()?;
                self.skip_whitespace_and_comma();
                let sweep = self.parse_flag()?;
                self.skip_whitespace_and_comma();
                let x = self.parse_number()?;
                self.skip_whitespace_and_comma();
                let y = self.parse_number()?;
                Ok(Command::Arc {
                    rel,
                    rx,
                    ry,
                    x_axis_rotation,
                    large_arc,
                    sweep,
                    x,
                    y,
                })
            }
            'z' => Ok(Command::ClosePath),
            _ => Err(SavageError::InvalidPath(format!(
                "Unknown command: {}",
                cmd
            ))),
        }
    }

    fn parse_number(&mut self) -> Result<f64, SavageError> {
        self.skip_whitespace_and_comma();

        let start = self.pos;

        // Optional sign
        if self.peek() == Some('-') || self.peek() == Some('+') {
            self.next();
        }

        // Integer part
        while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.next();
        }

        // Decimal part
        if self.peek() == Some('.') {
            self.next();
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.next();
            }
        }

        // Exponent
        if self.peek() == Some('e') || self.peek() == Some('E') {
            self.next();
            if self.peek() == Some('-') || self.peek() == Some('+') {
                self.next();
            }
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.next();
            }
        }

        let s = &self.input[start..self.pos];
        if s.is_empty() {
            return Err(SavageError::InvalidPath("Expected number".into()));
        }

        s.parse()
            .map_err(|_| SavageError::InvalidPath(format!("Invalid number: {}", s)))
    }

    fn parse_flag(&mut self) -> Result<bool, SavageError> {
        self.skip_whitespace_and_comma();
        match self.next() {
            Some('0') => Ok(false),
            Some('1') => Ok(true),
            Some(c) => Err(SavageError::InvalidPath(format!(
                "Expected flag (0 or 1), got: {}",
                c
            ))),
            None => Err(SavageError::InvalidPath("Expected flag".into())),
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek()
            .map(|c| c.is_ascii_whitespace())
            .unwrap_or(false)
        {
            self.next();
        }
    }

    fn skip_whitespace_and_comma(&mut self) {
        self.skip_whitespace();
        if self.peek() == Some(',') {
            self.next();
        }
        self.skip_whitespace();
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_path() {
        let path = parse_path("M10 20 L30 40").unwrap();
        assert_eq!(path.commands.len(), 2);
    }

    #[test]
    fn test_parse_relative_path() {
        let path = parse_path("m10,20 l30,40").unwrap();
        assert_eq!(path.commands.len(), 2);
        assert!(matches!(path.commands[0], Command::MoveTo { rel: true, .. }));
    }

    #[test]
    fn test_parse_implicit_lineto() {
        let path = parse_path("M10 20 30 40").unwrap();
        assert_eq!(path.commands.len(), 2);
        assert!(matches!(path.commands[1], Command::LineTo { .. }));
    }

    #[test]
    fn test_parse_arc() {
        let path = parse_path("A 10 20 30 1 0 40 50").unwrap();
        assert_eq!(path.commands.len(), 1);
        if let Command::Arc {
            large_arc, sweep, ..
        } = &path.commands[0]
        {
            assert!(*large_arc);
            assert!(!*sweep);
        } else {
            panic!("Expected Arc command");
        }
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(0.0, 2), "0");
        assert_eq!(format_number(1.0, 2), "1");
        assert_eq!(format_number(1.5, 2), "1.5");
        assert_eq!(format_number(1.50, 2), "1.5");
        assert_eq!(format_number(0.5, 2), ".5");
        assert_eq!(format_number(-0.5, 2), "-.5");
        assert_eq!(format_number(1.234, 2), "1.23");
        assert_eq!(format_number(1.235, 2), "1.24"); // rounding
    }

    #[test]
    fn test_serialize_path() {
        let path = parse_path("M 10.00 20.00 L 30.00 40.00 Z").unwrap();
        let out = serialize_path(&path, 0);
        // implicit L after M, so "30 40" follows
        assert_eq!(out, "M10 20 30 40z");
    }

    #[test]
    fn test_serialize_compact() {
        let path = parse_path("M 0.5 0.5 L -0.5 -0.5").unwrap();
        let out = serialize_path(&path, 1);
        // .5 .5 need space between (both start with .), -.5 doesn't need space before -
        assert_eq!(out, "M.5 .5-.5-.5");
    }
}
