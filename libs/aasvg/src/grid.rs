//! Grid representation for ASCII art diagrams.
//!
//! The grid provides character access and tracks which cells have been
//! consumed by path/decoration finding.

// Many methods are provided for library consumers but not used internally
#![allow(dead_code)]

use crate::chars::*;

/// 2D grid of characters with "used" tracking
pub struct Grid {
    /// Characters in the grid (row-major order)
    chars: Vec<Vec<char>>,
    /// Track which cells have been consumed
    used: Vec<Vec<bool>>,
    /// Grid width (longest line)
    pub width: usize,
    /// Grid height (number of lines)
    pub height: usize,
}

impl Grid {
    /// Create a grid from a diagram string
    pub fn new(input: &str) -> Self {
        let input = preprocess(input);
        let lines: Vec<&str> = input.lines().collect();

        let height = lines.len();
        let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);

        let mut chars = Vec::with_capacity(height);
        let mut used = Vec::with_capacity(height);

        for line in &lines {
            let mut row: Vec<char> = line.chars().collect();
            // Pad to width
            while row.len() < width {
                row.push(' ');
            }
            chars.push(row);
            used.push(vec![false; width]);
        }

        Self {
            chars,
            used,
            width,
            height,
        }
    }

    /// Get the character at position (x, y), or space if out of bounds
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> char {
        if x < 0 || y < 0 {
            return ' ';
        }
        let x = x as usize;
        let y = y as usize;
        if y >= self.height || x >= self.width {
            return ' ';
        }
        self.chars[y][x]
    }

    /// Mark a cell as used (consumed by path/decoration finding)
    pub fn set_used(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 {
            return;
        }
        let x = x as usize;
        let y = y as usize;
        if y < self.height && x < self.width {
            self.used[y][x] = true;
        }
    }

    /// Check if a cell has been used
    pub fn is_used(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let x = x as usize;
        let y = y as usize;
        if y >= self.height || x >= self.width {
            return false;
        }
        self.used[y][x]
    }

    // ========================================================================
    // Line detection at positions
    // ========================================================================

    /// Check if there's a solid vertical line at the given position
    pub fn is_solid_v_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_solid_v_line(c)
    }

    /// Check if there's a double vertical line at the given position
    pub fn is_double_v_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_double_v_line(c)
    }

    /// Check if there's a solid horizontal line at the given position
    pub fn is_solid_h_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_solid_h_line(c)
    }

    /// Check if there's a squiggle horizontal line at the given position
    pub fn is_squiggle_h_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_squiggle_h_line(c)
    }

    /// Check if there's a double horizontal line at the given position
    pub fn is_double_h_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_double_h_line(c)
    }

    /// Check if there's any horizontal line at the given position
    pub fn is_any_h_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_any_h_line(c)
    }

    /// Check if there's a solid backslash diagonal at the given position
    pub fn is_solid_b_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_solid_b_line(c)
    }

    /// Check if there's a solid forward slash diagonal at the given position
    pub fn is_solid_d_line_at(&self, x: i32, y: i32) -> bool {
        let c = self.get(x, y);
        is_solid_d_line(c)
    }

    // ========================================================================
    // Vertical line detection (looking at adjacent chars)
    // ========================================================================

    /// Check if there's a vertical line going through (x, y)
    /// Uses the given predicate to check the character
    pub fn is_v_line_at_with<F>(&self, x: i32, y: i32, pred: F) -> bool
    where
        F: Fn(char) -> bool,
    {
        let c = self.get(x, y);
        if !pred(c) {
            return false;
        }

        // Check for vertical continuation or vertex connection
        let above = self.get(x, y - 1);
        let below = self.get(x, y + 1);

        // Must connect to something above or below
        pred(above)
            || pred(below)
            || is_top_vertex(above)
            || is_bottom_vertex(below)
            || is_arrow_head(above)
            || is_arrow_head(below)
    }

    // ========================================================================
    // Horizontal line detection (looking at adjacent chars)
    // ========================================================================

    /// Check if there's a horizontal line going through (x, y)
    /// Uses the given predicate to check the character
    pub fn is_h_line_at_with<F>(&self, x: i32, y: i32, pred: F) -> bool
    where
        F: Fn(char) -> bool,
    {
        let c = self.get(x, y);
        if !pred(c) {
            return false;
        }

        // Check for horizontal continuation or vertex connection
        let left = self.get(x - 1, y);
        let right = self.get(x + 1, y);

        // Must connect to something left or right
        pred(left) || pred(right) || is_vertex(left) || is_vertex(right)
    }

    // ========================================================================
    // Text extraction
    // ========================================================================

    /// Find the starting x position of a text run at (start_x, y)
    /// Returns None if there's no text at this position
    pub fn text_start(&self, start_x: i32, y: i32, _spaces: u32) -> Option<i32> {
        let mut x = start_x;
        while x < self.width as i32 {
            let c = self.get(x, y);
            if c != ' ' && !self.is_used(x, y) {
                return Some(x);
            }
            x += 1;
        }
        None
    }

    /// Extract a text string starting at (x, y)
    /// Marks the characters as used
    /// Stops after `spaces` consecutive spaces
    pub fn extract_text(&mut self, start_x: i32, y: i32, spaces: u32) -> String {
        let mut result = String::new();
        let mut x = start_x;
        let mut space_count = 0;

        while x < self.width as i32 {
            let c = self.get(x, y);

            if c == ' ' {
                space_count += 1;
                if space_count >= spaces && spaces > 0 {
                    // Trim trailing spaces
                    while result.ends_with(' ') {
                        result.pop();
                    }
                    break;
                }
                result.push(c);
            } else if self.is_used(x, y) {
                // Hit a used cell, stop
                while result.ends_with(' ') {
                    result.pop();
                }
                break;
            } else {
                space_count = 0;
                result.push(c);
                self.set_used(x, y);
            }
            x += 1;
        }

        // Trim trailing spaces
        while result.ends_with(' ') {
            result.pop();
        }

        result
    }
}

/// Preprocess the diagram string:
/// - Equalize line lengths (pad with spaces)
/// - Remove common leading whitespace
/// - Hide marker characters in text (o, v, V)
fn preprocess(input: &str) -> String {
    let input = remove_leading_space(input);
    let input = equalize_line_lengths(&input);
    hide_markers(&input)
}

/// Remove common leading whitespace from all lines
fn remove_leading_space(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();

    // Find minimum leading spaces (ignoring empty lines)
    let min_spaces = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    if min_spaces == 0 {
        return input.to_string();
    }

    lines
        .iter()
        .map(|l| {
            if l.len() >= min_spaces {
                &l[min_spaces..]
            } else {
                ""
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pad all lines to the same length
fn equalize_line_lengths(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let max_len = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);

    lines
        .iter()
        .map(|l| {
            let len = l.chars().count();
            if len < max_len {
                let padding = " ".repeat(max_len - len);
                format!("{}{}", l, padding)
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Hide 'o', 'v', 'V' characters that appear to be part of text
/// (surrounded by letters, not connected to lines)
fn hide_markers(input: &str) -> String {
    let lines: Vec<Vec<char>> = input.lines().map(|l| l.chars().collect()).collect();
    let height = lines.len();

    let get = |x: i32, y: i32| -> char {
        if y < 0 || y >= height as i32 {
            return ' ';
        }
        let row = &lines[y as usize];
        if x < 0 || x >= row.len() as i32 {
            return ' ';
        }
        row[x as usize]
    };

    let is_letter = |c: char| -> bool { c.is_ascii_alphabetic() };

    let mut result: Vec<Vec<char>> = lines.clone();

    for y in 0..height {
        for x in 0..lines[y].len() {
            let c = lines[y][x];
            let xi = x as i32;
            let yi = y as i32;

            // Check if o, v, V is part of a word (surrounded by letters)
            if c == 'o' || c == 'v' || c == 'V' {
                let left = get(xi - 1, yi);
                let right = get(xi + 1, yi);

                // If surrounded by letters on left or right, it's part of text
                if is_letter(left) || is_letter(right) {
                    // Replace with a placeholder that won't be detected as decoration
                    // Use a private use Unicode character
                    result[y][x] = match c {
                        'o' => '\u{E000}', // Private use for 'o'
                        'v' => '\u{E001}', // Private use for 'v'
                        'V' => '\u{E002}', // Private use for 'V'
                        _ => c,
                    };
                }
            }
        }
    }

    result
        .iter()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Unhide previously hidden marker characters
pub fn unhide_markers(input: &str) -> String {
    input
        .replace('\u{E000}', "o")
        .replace('\u{E001}', "v")
        .replace('\u{E002}', "V")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_creation() {
        let grid = Grid::new("+--+\n|  |\n+--+");
        assert_eq!(grid.width, 4);
        assert_eq!(grid.height, 3);
        assert_eq!(grid.get(0, 0), '+');
        assert_eq!(grid.get(1, 0), '-');
        assert_eq!(grid.get(0, 1), '|');
        assert_eq!(grid.get(1, 1), ' ');
    }

    #[test]
    fn test_grid_out_of_bounds() {
        let grid = Grid::new("AB\nCD");
        assert_eq!(grid.get(-1, 0), ' ');
        assert_eq!(grid.get(0, -1), ' ');
        assert_eq!(grid.get(10, 0), ' ');
        assert_eq!(grid.get(0, 10), ' ');
    }

    #[test]
    fn test_grid_used_tracking() {
        let mut grid = Grid::new("AB");
        assert!(!grid.is_used(0, 0));
        grid.set_used(0, 0);
        assert!(grid.is_used(0, 0));
        assert!(!grid.is_used(1, 0));
    }

    #[test]
    fn test_remove_leading_space() {
        let input = "  abc\n  def";
        let result = remove_leading_space(input);
        assert_eq!(result, "abc\ndef");
    }

    #[test]
    fn test_equalize_line_lengths() {
        let input = "ab\na";
        let result = equalize_line_lengths(input);
        assert_eq!(result, "ab\na ");
    }

    #[test]
    fn test_text_extraction() {
        // Use text without o, v, V which get hidden as markers
        let mut grid = Grid::new("Test String");
        let text = grid.extract_text(0, 0, 2);
        assert_eq!(text, "Test String");
    }
}
