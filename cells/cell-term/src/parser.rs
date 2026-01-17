//! ANSI escape code parser
//!
//! Ported from libterm in cove.

use anstyle_parse::Perform;

/// Color representation
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    #[default]
    Default,
    Classic(ClassicColor),
    Rgb(u8, u8, u8),
    Ansi256(u8),
}

impl From<ClassicColor> for Color {
    fn from(c: ClassicColor) -> Self {
        Self::Classic(c)
    }
}

/// Classic 16 ANSI colors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassicColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    LightBlack,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    LightWhite,
}

impl ClassicColor {
    /// Get the tag suffix for this color (e.g., "blk", "red", "lred")
    pub fn tag_suffix(&self) -> &'static str {
        match self {
            ClassicColor::Black => "blk",
            ClassicColor::Red => "red",
            ClassicColor::Green => "grn",
            ClassicColor::Yellow => "ylw",
            ClassicColor::Blue => "blu",
            ClassicColor::Magenta => "mag",
            ClassicColor::Cyan => "cyn",
            ClassicColor::White => "wht",
            ClassicColor::LightBlack => "lblk",
            ClassicColor::LightRed => "lred",
            ClassicColor::LightGreen => "lgrn",
            ClassicColor::LightYellow => "lylw",
            ClassicColor::LightBlue => "lblu",
            ClassicColor::LightMagenta => "lmag",
            ClassicColor::LightCyan => "lcyn",
            ClassicColor::LightWhite => "lwht",
        }
    }
}

/// Text weight
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Faint,
    #[default]
    Normal,
    Bold,
}

/// Text decoration
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decoration {
    #[default]
    None,
    Underline,
    Strikethrough,
}

/// Font style
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}

/// Combined style for a character
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub weight: Weight,
    pub decoration: Decoration,
    pub font_style: FontStyle,
}

/// A character with its style
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyledChar {
    pub style: Style,
    pub c: char,
}

impl Default for StyledChar {
    fn default() -> Self {
        Self {
            style: Default::default(),
            c: ' ',
        }
    }
}

/// Screen state - tracks the virtual terminal
#[derive(Default)]
pub struct Screen {
    pub lines: Vec<Vec<StyledChar>>,
    pub style: Style,
    pub row: usize,
    pub col: usize,
}

impl Screen {
    pub fn line_mut(&mut self) -> &mut Vec<StyledChar> {
        while self.lines.len() <= self.row {
            self.lines.push(Default::default());
        }
        &mut self.lines[self.row]
    }

    pub fn char_mut(&mut self) -> &mut StyledChar {
        let col = self.col;
        let line = self.line_mut();
        while line.len() <= col {
            line.push(Default::default());
        }
        &mut line[col]
    }

    fn set_color_from<'a>(
        &mut self,
        first: &'a [u16],
        params: &mut impl Iterator<Item = &'a [u16]>,
    ) {
        match first[0] {
            // Classic foreground colors
            30 => self.style.fg = ClassicColor::Black.into(),
            31 => self.style.fg = ClassicColor::Red.into(),
            32 => self.style.fg = ClassicColor::Green.into(),
            33 => self.style.fg = ClassicColor::Yellow.into(),
            34 => self.style.fg = ClassicColor::Blue.into(),
            35 => self.style.fg = ClassicColor::Magenta.into(),
            36 => self.style.fg = ClassicColor::Cyan.into(),
            37 => self.style.fg = ClassicColor::White.into(),
            90 => self.style.fg = ClassicColor::LightBlack.into(),
            91 => self.style.fg = ClassicColor::LightRed.into(),
            92 => self.style.fg = ClassicColor::LightGreen.into(),
            93 => self.style.fg = ClassicColor::LightYellow.into(),
            94 => self.style.fg = ClassicColor::LightBlue.into(),
            95 => self.style.fg = ClassicColor::LightMagenta.into(),
            96 => self.style.fg = ClassicColor::LightCyan.into(),
            97 => self.style.fg = ClassicColor::LightWhite.into(),

            // Classic background colors
            40 => self.style.bg = ClassicColor::Black.into(),
            41 => self.style.bg = ClassicColor::Red.into(),
            42 => self.style.bg = ClassicColor::Green.into(),
            43 => self.style.bg = ClassicColor::Yellow.into(),
            44 => self.style.bg = ClassicColor::Blue.into(),
            45 => self.style.bg = ClassicColor::Magenta.into(),
            46 => self.style.bg = ClassicColor::Cyan.into(),
            47 => self.style.bg = ClassicColor::White.into(),
            100 => self.style.bg = ClassicColor::LightBlack.into(),
            101 => self.style.bg = ClassicColor::LightRed.into(),
            102 => self.style.bg = ClassicColor::LightGreen.into(),
            103 => self.style.bg = ClassicColor::LightYellow.into(),
            104 => self.style.bg = ClassicColor::LightBlue.into(),
            105 => self.style.bg = ClassicColor::LightMagenta.into(),
            106 => self.style.bg = ClassicColor::LightCyan.into(),
            107 => self.style.bg = ClassicColor::LightWhite.into(),

            // Extended colors (256 and 24-bit)
            38 | 48 => {
                let is_fg = first[0] == 38;
                if let Some(kind) = params.next() {
                    if kind[0] == 5 {
                        // 256 color mode
                        if let Some(color_param) = params.next() {
                            let color = color_param[0] as u8;
                            if is_fg {
                                self.style.fg = Color::Ansi256(color);
                            } else {
                                self.style.bg = Color::Ansi256(color);
                            }
                        }
                    } else if kind[0] == 2 {
                        // 24-bit RGB
                        let r = params.next().map(|p| p[0] as u8).unwrap_or(0);
                        let g = params.next().map(|p| p[0] as u8).unwrap_or(0);
                        let b = params.next().map(|p| p[0] as u8).unwrap_or(0);
                        if is_fg {
                            self.style.fg = Color::Rgb(r, g, b);
                        } else {
                            self.style.bg = Color::Rgb(r, g, b);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// ANSI parser that implements the Perform trait
#[derive(Default)]
pub struct Performer {
    pub screen: Screen,
    alt_screen: Screen,
    saved_cursor_pos: Option<(usize, usize)>,
}

impl Perform for Performer {
    fn print(&mut self, c: char) {
        let style = self.screen.style;
        let cm = self.screen.char_mut();
        cm.c = c;
        cm.style = style;
        self.screen.col += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {} // bell, ignore
            0x08 => {
                // backspace
                if self.screen.col > 0 {
                    self.screen.col -= 1;
                }
            }
            0x09 => {
                // tab
                let col = self.screen.col;
                self.screen.col = col + (8 - (col % 8));
            }
            0x0A => {
                // line feed
                self.screen.row += 1;
            }
            0x0C => {} // form feed, ignore
            0x0D => {
                // carriage return
                self.screen.col = 0;
            }
            _ => {}
        }
    }

    fn hook(
        &mut self,
        _params: &anstyle_parse::Params,
        _intermediates: &[u8],
        _ignore: bool,
        _action: u8,
    ) {
    }

    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

    fn csi_dispatch(
        &mut self,
        params: &anstyle_parse::Params,
        intermediates: &[u8],
        ignore: bool,
        action: u8,
    ) {
        if ignore || !intermediates.is_empty() {
            return;
        }

        match action {
            b'm' => {
                // SGR - Select Graphic Rendition
                let mut params = params.iter();
                while let Some(param1) = params.next() {
                    match param1[0] {
                        0 => self.screen.style = Default::default(),
                        1 => self.screen.style.weight = Weight::Bold,
                        2 => self.screen.style.weight = Weight::Faint,
                        3 => self.screen.style.font_style = FontStyle::Italic,
                        4 => self.screen.style.decoration = Decoration::Underline,
                        7 => std::mem::swap(&mut self.screen.style.fg, &mut self.screen.style.bg),
                        9 => self.screen.style.decoration = Decoration::Strikethrough,
                        22 => self.screen.style.weight = Weight::Normal,
                        23 => self.screen.style.font_style = FontStyle::Normal,
                        24 => self.screen.style.decoration = Decoration::None,
                        27 => std::mem::swap(&mut self.screen.style.fg, &mut self.screen.style.bg),
                        29 => self.screen.style.decoration = Decoration::None,
                        30..=37 | 90..=97 => self.screen.set_color_from(param1, &mut params),
                        38 => self.screen.set_color_from(param1, &mut params),
                        39 => self.screen.style.fg = Color::Default,
                        40..=47 | 100..=107 => self.screen.set_color_from(param1, &mut params),
                        48 => self.screen.set_color_from(param1, &mut params),
                        49 => self.screen.style.bg = Color::Default,
                        _ => {}
                    }
                }
            }
            b'h' | b'l' => {
                // Mode set/reset
                let param1 = params.iter().next().unwrap_or(&[1])[0] as usize;
                if param1 == 1049 {
                    // Alternate screen buffer
                    std::mem::swap(&mut self.screen, &mut self.alt_screen);
                }
            }
            b'A' => {
                // Move up
                let n = params.iter().next().unwrap_or(&[1])[0].max(1) as usize;
                self.screen.row = self.screen.row.saturating_sub(n);
            }
            b'B' => {
                // Move down
                let n = params.iter().next().unwrap_or(&[1])[0].max(1) as usize;
                self.screen.row += n;
            }
            b'C' => {
                // Move right
                let n = params.iter().next().unwrap_or(&[1])[0].max(1) as usize;
                self.screen.col += n;
            }
            b'D' => {
                // Move left
                let n = params.iter().next().unwrap_or(&[1])[0].max(1) as usize;
                self.screen.col = self.screen.col.saturating_sub(n);
            }
            b'E' => {
                // Move down and to column 1
                let n = params.iter().next().unwrap_or(&[1])[0] as usize;
                self.screen.row += n;
                self.screen.col = 0;
            }
            b'F' => {
                // Move up and to column 1
                let n = params.iter().next().unwrap_or(&[1])[0] as usize;
                self.screen.row = self.screen.row.saturating_sub(n);
                self.screen.col = 0;
            }
            b'G' => {
                // Cursor horizontal absolute
                let col = params.iter().next().unwrap_or(&[1])[0] as usize;
                self.screen.col = col.saturating_sub(1);
            }
            b'H' | b'f' => {
                // Cursor position
                let mut p = params.iter();
                let row = p.next().unwrap_or(&[1])[0] as usize;
                let col = p.next().unwrap_or(&[1])[0] as usize;
                self.screen.row = row.saturating_sub(1);
                self.screen.col = col.saturating_sub(1);
            }
            b'J' => {
                // Erase in display
                let mode = params.iter().next().unwrap_or(&[0])[0];
                match mode {
                    0 => {
                        // Clear from cursor to end
                        let col = self.screen.col;
                        for row in self.screen.row..self.screen.lines.len() {
                            if row >= self.screen.lines.len() {
                                continue;
                            }
                            let line = &mut self.screen.lines[row];
                            let start = if row == self.screen.row { col } else { 0 };
                            for i in start..line.len() {
                                line[i].c = ' ';
                            }
                        }
                    }
                    1 => {
                        // Clear from start to cursor
                        let col = self.screen.col;
                        for row in 0..=self.screen.row {
                            if row >= self.screen.lines.len() {
                                continue;
                            }
                            let line = &mut self.screen.lines[row];
                            let end = if row == self.screen.row {
                                col + 1
                            } else {
                                line.len()
                            };
                            for i in 0..end.min(line.len()) {
                                line[i].c = ' ';
                            }
                        }
                    }
                    2 | 3 => {
                        // Clear entire screen
                        for line in &mut self.screen.lines {
                            for sc in line {
                                sc.c = ' ';
                            }
                        }
                    }
                    _ => {}
                }
            }
            b'K' => {
                // Erase in line
                let mode = params.iter().next().unwrap_or(&[0])[0];
                let col = self.screen.col;
                let line = self.screen.line_mut();
                match mode {
                    0 => {
                        for i in col..line.len() {
                            line[i].c = ' ';
                        }
                    }
                    1 => {
                        for i in 0..=col.min(line.len().saturating_sub(1)) {
                            line[i].c = ' ';
                        }
                    }
                    2 => {
                        for sc in line {
                            sc.c = ' ';
                        }
                    }
                    _ => {}
                }
            }
            b'P' => {
                // Delete character
                let n = params.iter().next().unwrap_or(&[1])[0].max(1) as usize;
                let col = self.screen.col;
                let line = self.screen.line_mut();
                if col < line.len() {
                    let end = (col + n).min(line.len());
                    line.drain(col..end);
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => {
                // Save cursor
                self.saved_cursor_pos = Some((self.screen.row, self.screen.col));
            }
            b'8' => {
                // Restore cursor
                if let Some((row, col)) = self.saved_cursor_pos {
                    self.screen.row = row;
                    self.screen.col = col;
                }
            }
            _ => {}
        }
    }
}

/// Parse ANSI-encoded bytes into a Performer with screen state
pub fn parse(data: &[u8]) -> Performer {
    let mut parser = anstyle_parse::Parser::<anstyle_parse::Utf8Parser>::new();
    let mut performer = Performer::default();

    for &byte in data {
        parser.advance(&mut performer, byte);
    }

    performer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::render;

    #[test]
    fn test_basic_colors() {
        let input = b"\x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert!(html.contains("<t-fred>"));
        assert!(html.contains("<t-fgrn>"));
    }

    #[test]
    fn test_bold() {
        let input = b"\x1b[1mbold\x1b[0m";
        let performer = parse(input);
        let html = render(&performer.screen);
        assert!(html.contains("<t-b>"));
    }
}
