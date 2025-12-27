//! Character classification functions for ASCII art diagram parsing.
//!
//! These functions identify the role of each character in the diagram:
//! lines, vertices, arrows, decorations, etc.

// Many functions are provided for library consumers but not used internally
#![allow(dead_code)]

/// Arrow head characters that indicate line direction
pub const ARROW_HEAD_CHARS: &str = ">v<^V";

/// Point/dot decoration characters
pub const POINT_CHARS: &str = "o*◌○◍●⊕";

/// Jump (bridge) characters for line crossings
pub const JUMP_CHARS: &str = "()";

/// Vertex characters that can connect lines in any direction
pub const UNDIRECTED_VERTEX_CHARS: &str = "+";

/// All vertex characters (connection points for lines)
pub const VERTEX_CHARS: &str = "+.',`";

/// Gray fill characters (various shading levels)
pub const GRAY_CHARS: &str = "▁▂▃█";

/// Triangle decoration characters
pub const TRI_CHARS: &str = "◢◣◤◥";

// ============================================================================
// Vertex classification
// ============================================================================

/// Returns true if the character is part of the line network (a vertex/junction)
#[inline]
pub fn is_vertex(c: char) -> bool {
    VERTEX_CHARS.contains(c)
}

/// Returns true if the character is an undirected vertex (+)
/// These can connect lines in all four directions
#[inline]
pub fn is_undirected_vertex(c: char) -> bool {
    c == '+'
}

/// Returns true if the character can serve as a top vertex (., , or +)
/// These connect to lines going down
#[inline]
pub fn is_top_vertex(c: char) -> bool {
    c == '.' || c == ',' || c == '+'
}

/// Returns true if the character can serve as a bottom vertex (', `, or +)
/// These connect to lines going up
#[inline]
pub fn is_bottom_vertex(c: char) -> bool {
    c == '\'' || c == '`' || c == '+'
}

/// Returns true if the character is a top vertex or an upward arrow (^)
#[inline]
pub fn is_top_vertex_or_decoration(c: char) -> bool {
    is_top_vertex(c) || c == '^'
}

/// Returns true if the character is a bottom vertex or a downward arrow (v, V)
#[inline]
pub fn is_bottom_vertex_or_decoration(c: char) -> bool {
    is_bottom_vertex(c) || c == 'v' || c == 'V'
}

/// Returns true if the character is a vertex, left arrow (<), or point
#[inline]
pub fn is_vertex_or_left_decoration(c: char) -> bool {
    is_vertex(c) || c == '<' || is_point(c)
}

/// Returns true if the character is a vertex, right arrow (>), or point
#[inline]
pub fn is_vertex_or_right_decoration(c: char) -> bool {
    is_vertex(c) || c == '>' || is_point(c)
}

// ============================================================================
// Line classification
// ============================================================================

/// Returns true if the character is a solid horizontal line segment
#[inline]
pub fn is_solid_h_line(c: char) -> bool {
    c == '-' || c == '─' || c == '+' || c == '(' || c == ')'
}

/// Returns true if the character is a squiggle/wave horizontal line segment
#[inline]
pub fn is_squiggle_h_line(c: char) -> bool {
    c == '~' || c == '+' || c == '(' || c == ')'
}

/// Returns true if the character is a double horizontal line segment
#[inline]
pub fn is_double_h_line(c: char) -> bool {
    c == '=' || c == '═' || c == '+' || c == '(' || c == ')'
}

/// Returns true if the character is any horizontal line type
#[inline]
pub fn is_any_h_line(c: char) -> bool {
    is_solid_h_line(c) || is_squiggle_h_line(c) || is_double_h_line(c)
}

/// Returns true if the character is a solid vertical line segment
#[inline]
pub fn is_solid_v_line(c: char) -> bool {
    c == '|' || c == '│' || c == '+'
}

/// Returns true if the character is a double vertical line segment
#[inline]
pub fn is_double_v_line(c: char) -> bool {
    c == '║' || c == '+'
}

/// Returns true if the character is a forward slash diagonal (/)
#[inline]
pub fn is_solid_d_line(c: char) -> bool {
    c == '/' || c == '╱'
}

/// Returns true if the character is a backslash diagonal (\)
#[inline]
pub fn is_solid_b_line(c: char) -> bool {
    c == '\\' || c == '╲'
}

// ============================================================================
// Decoration classification
// ============================================================================

/// Returns true if the character is a gray fill character
#[inline]
pub fn is_gray(c: char) -> bool {
    GRAY_CHARS.contains(c)
}

/// Returns true if the character is a triangle decoration
#[inline]
pub fn is_tri(c: char) -> bool {
    TRI_CHARS.contains(c)
}

/// Returns true if the character is an arrow head
#[inline]
pub fn is_arrow_head(c: char) -> bool {
    ARROW_HEAD_CHARS.contains(c)
}

/// Returns true if the character is a point/dot decoration
#[inline]
pub fn is_point(c: char) -> bool {
    POINT_CHARS.contains(c)
}

/// Returns true if the character is a jump (bridge) marker
#[inline]
pub fn is_jump(c: char) -> bool {
    c == '(' || c == ')'
}

/// Returns true if the character is any kind of decoration
#[inline]
pub fn is_decoration(c: char) -> bool {
    is_arrow_head(c) || is_point(c) || is_gray(c) || is_tri(c)
}

// ============================================================================
// Text/identifier helpers
// ============================================================================

/// Returns true if the character is an ASCII letter (a-z, A-Z)
#[inline]
pub fn is_ascii_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
}

/// Gray level for fill characters (0-255)
pub fn gray_level(c: char) -> u8 {
    match c {
        '▁' => 64,
        '▂' => 128,
        '▃' => 191,
        '█' => 255,
        _ => 0,
    }
}

/// Triangle rotation angle in degrees
pub fn tri_angle(c: char) -> f64 {
    match c {
        '◢' => 0.0,
        '◣' => 90.0,
        '◤' => 180.0,
        '◥' => 270.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertex_detection() {
        assert!(is_vertex('+'));
        assert!(is_vertex('.'));
        assert!(is_vertex('\''));
        assert!(is_vertex(','));
        assert!(is_vertex('`'));
        assert!(!is_vertex('-'));
        assert!(!is_vertex('|'));
    }

    #[test]
    fn test_line_detection() {
        assert!(is_solid_h_line('-'));
        assert!(is_solid_h_line('─'));
        assert!(is_solid_h_line('+'));
        assert!(!is_solid_h_line('|'));

        assert!(is_solid_v_line('|'));
        assert!(is_solid_v_line('│'));
        assert!(is_solid_v_line('+'));
        assert!(!is_solid_v_line('-'));

        assert!(is_solid_d_line('/'));
        assert!(is_solid_b_line('\\'));
    }

    #[test]
    fn test_arrow_detection() {
        assert!(is_arrow_head('>'));
        assert!(is_arrow_head('<'));
        assert!(is_arrow_head('^'));
        assert!(is_arrow_head('v'));
        assert!(is_arrow_head('V'));
        assert!(!is_arrow_head('-'));
    }

    #[test]
    fn test_point_detection() {
        assert!(is_point('o'));
        assert!(is_point('*'));
        assert!(is_point('●'));
        assert!(is_point('○'));
        assert!(!is_point('+'));
    }

    #[test]
    fn test_gray_levels() {
        assert_eq!(gray_level('▁'), 64);
        assert_eq!(gray_level('▂'), 128);
        assert_eq!(gray_level('▃'), 191);
        assert_eq!(gray_level('█'), 255);
        assert_eq!(gray_level('x'), 0);
    }
}
