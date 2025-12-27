//! Path and decoration finding algorithms.
//!
//! This module scans the grid to find all lines, curves, and decorations.

use crate::chars::*;
use crate::decoration::*;
use crate::grid::Grid;
use crate::path::*;

/// Find all paths (lines and curves) in the grid
pub fn find_paths(grid: &mut Grid, paths: &mut PathSet) {
    find_solid_vertical_lines(grid, paths);
    find_double_vertical_lines(grid, paths);
    find_solid_horizontal_lines(grid, paths);
    find_squiggle_horizontal_lines(grid, paths);
    find_double_horizontal_lines(grid, paths);
    find_backslash_diagonals(grid, paths);
    find_forward_slash_diagonals(grid, paths);
    find_curved_corners(grid, paths);
    find_underscore_lines(grid, paths);
}

/// Find all decorations (arrows, points, etc.) in the grid
pub fn find_decorations(grid: &mut Grid, paths: &PathSet, decorations: &mut DecorationSet) {
    find_arrow_heads(grid, paths, decorations);
    find_points(grid, paths, decorations);
    find_jumps(grid, paths, decorations);
    find_gray_fills(grid, decorations);
    find_triangles(grid, decorations);
}

// ============================================================================
// Vertical line finding
// ============================================================================

fn find_solid_vertical_lines(grid: &mut Grid, paths: &mut PathSet) {
    for x in 0..grid.width as i32 {
        let mut y = 0;
        while y < grid.height as i32 {
            if is_solid_v_line(grid.get(x, y)) {
                let start_y = y;
                while y < grid.height as i32 && is_solid_v_line(grid.get(x, y)) {
                    grid.set_used(x, y);
                    y += 1;
                }
                let end_y = y - 1;

                if end_y > start_y {
                    // Adjust endpoints for vertices
                    let mut adj_start_y = start_y;
                    let mut adj_end_y = end_y;

                    // Check if we should extend to connect with vertices
                    if is_top_vertex(grid.get(x, start_y - 1)) {
                        adj_start_y = start_y - 1;
                    }
                    if is_bottom_vertex(grid.get(x, end_y + 1)) {
                        adj_end_y = end_y + 1;
                    }

                    let path = Path::line_from_grid(x, adj_start_y, x, adj_end_y);
                    paths.insert(path);
                }
            } else {
                y += 1;
            }
        }
    }
}

fn find_double_vertical_lines(grid: &mut Grid, paths: &mut PathSet) {
    for x in 0..grid.width as i32 {
        let mut y = 0;
        while y < grid.height as i32 {
            let c = grid.get(x, y);
            if c == '║' {
                let start_y = y;
                while y < grid.height as i32 && grid.get(x, y) == '║' {
                    grid.set_used(x, y);
                    y += 1;
                }
                let end_y = y - 1;

                if end_y >= start_y {
                    let path = Path::line_from_grid(x, start_y, x, end_y).with_double(true);
                    paths.insert(path);
                }
            } else {
                y += 1;
            }
        }
    }
}

// ============================================================================
// Horizontal line finding
// ============================================================================

fn find_solid_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            let c = grid.get(x, y);
            if c == '-' || c == '─' {
                let start_x = x;
                while x < grid.width as i32 {
                    let c = grid.get(x, y);
                    if c == '-' || c == '─' || c == '+' {
                        grid.set_used(x, y);
                        x += 1;
                    } else {
                        break;
                    }
                }
                let end_x = x - 1;

                if end_x > start_x {
                    // Adjust for vertices
                    let mut adj_start_x = start_x;
                    let mut adj_end_x = end_x;

                    if is_vertex(grid.get(start_x - 1, y)) {
                        adj_start_x = start_x - 1;
                    }
                    if is_vertex(grid.get(end_x + 1, y)) {
                        adj_end_x = end_x + 1;
                    }

                    let path = Path::line_from_grid(adj_start_x, y, adj_end_x, y);
                    paths.insert(path);
                }
            } else {
                x += 1;
            }
        }
    }
}

fn find_squiggle_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            if grid.get(x, y) == '~' {
                let start_x = x;
                while x < grid.width as i32 && grid.get(x, y) == '~' {
                    grid.set_used(x, y);
                    x += 1;
                }
                let end_x = x - 1;

                if end_x > start_x {
                    let path = Path::line_from_grid(start_x, y, end_x, y).with_squiggle(true);
                    paths.insert(path);
                }
            } else {
                x += 1;
            }
        }
    }
}

fn find_double_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            let c = grid.get(x, y);
            if c == '=' || c == '═' {
                let start_x = x;
                while x < grid.width as i32 {
                    let c = grid.get(x, y);
                    if c == '=' || c == '═' {
                        grid.set_used(x, y);
                        x += 1;
                    } else {
                        break;
                    }
                }
                let end_x = x - 1;

                if end_x >= start_x {
                    let path = Path::line_from_grid(start_x, y, end_x, y).with_double(true);
                    paths.insert(path);
                }
            } else {
                x += 1;
            }
        }
    }
}

// ============================================================================
// Diagonal line finding
// ============================================================================

fn find_backslash_diagonals(grid: &mut Grid, paths: &mut PathSet) {
    // Scan diagonals from top-left to bottom-right
    let width = grid.width as i32;
    let height = grid.height as i32;

    // Start from each cell in first row and first column
    for start in 0..(width + height - 1) {
        let (start_x, start_y) = if start < width {
            (start, 0)
        } else {
            (0, start - width + 1)
        };

        let mut x = start_x;
        let mut y = start_y;

        while x < width && y < height {
            if is_solid_b_line(grid.get(x, y)) {
                let line_start_x = x;
                let line_start_y = y;

                while x < width && y < height && is_solid_b_line(grid.get(x, y)) {
                    grid.set_used(x, y);
                    x += 1;
                    y += 1;
                }

                let line_end_x = x - 1;
                let line_end_y = y - 1;

                if line_end_x > line_start_x {
                    let path = Path::line_from_grid(line_start_x, line_start_y, line_end_x, line_end_y);
                    paths.insert(path);
                }
            } else {
                x += 1;
                y += 1;
            }
        }
    }
}

fn find_forward_slash_diagonals(grid: &mut Grid, paths: &mut PathSet) {
    // Scan diagonals from top-right to bottom-left
    // Forward slash / connects lower-left to upper-right of a cell
    let width = grid.width as i32;
    let height = grid.height as i32;

    // Start from each cell in top row and right column
    for start in 0..(width + height - 1) {
        let (start_x, start_y) = if start < width {
            (width - 1 - start, 0) // Top edge, right to left
        } else {
            (width - 1, start - width + 1) // Right edge, top to bottom
        };

        let mut x = start_x;
        let mut y = start_y;

        // Move down-left (x decreases, y increases)
        while x >= 0 && y < height {
            if is_solid_d_line(grid.get(x, y)) {
                let line_start_x = x;
                let line_start_y = y;

                while x >= 0 && y < height && is_solid_d_line(grid.get(x, y)) {
                    grid.set_used(x, y);
                    x -= 1;
                    y += 1;
                }

                let line_end_x = x + 1;
                let line_end_y = y - 1;

                if line_start_x > line_end_x {
                    // For forward slash: start is top-right, end is bottom-left
                    // Create path from bottom-left to top-right for consistency
                    let path = Path::line_from_grid(line_end_x, line_end_y, line_start_x, line_start_y);
                    paths.insert(path);
                }
            } else {
                x -= 1;
                y += 1;
            }
        }
    }
}

// ============================================================================
// Curved corner finding
// ============================================================================

fn find_curved_corners(grid: &mut Grid, paths: &mut PathSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);

            // Check for curve patterns like -. .- -' '-
            if c == '.' || c == ',' {
                // Top vertex - curves down
                let left = grid.get(x - 1, y);
                let right = grid.get(x + 1, y);
                let below = grid.get(x, y + 1);

                // -. pattern (curve from left to down)
                if is_solid_h_line(left) && is_solid_v_line(below) {
                    let start = Vec2::from_grid(x, y).offset(-0.5, 0.0);
                    let end = Vec2::from_grid(x, y).offset(0.0, 0.5);
                    let ctrl1 = Vec2::from_grid(x, y);
                    let ctrl2 = Vec2::from_grid(x, y);
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                    grid.set_used(x, y);
                }

                // .- pattern (curve from right to down)
                if is_solid_h_line(right) && is_solid_v_line(below) {
                    let start = Vec2::from_grid(x, y).offset(0.5, 0.0);
                    let end = Vec2::from_grid(x, y).offset(0.0, 0.5);
                    let ctrl1 = Vec2::from_grid(x, y);
                    let ctrl2 = Vec2::from_grid(x, y);
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                    grid.set_used(x, y);
                }
            }

            if c == '\'' || c == '`' {
                // Bottom vertex - curves up
                let left = grid.get(x - 1, y);
                let right = grid.get(x + 1, y);
                let above = grid.get(x, y - 1);

                // -' pattern (curve from left to up)
                if is_solid_h_line(left) && is_solid_v_line(above) {
                    let start = Vec2::from_grid(x, y).offset(-0.5, 0.0);
                    let end = Vec2::from_grid(x, y).offset(0.0, -0.5);
                    let ctrl1 = Vec2::from_grid(x, y);
                    let ctrl2 = Vec2::from_grid(x, y);
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                    grid.set_used(x, y);
                }

                // '- pattern (curve from right to up)
                if is_solid_h_line(right) && is_solid_v_line(above) {
                    let start = Vec2::from_grid(x, y).offset(0.5, 0.0);
                    let end = Vec2::from_grid(x, y).offset(0.0, -0.5);
                    let ctrl1 = Vec2::from_grid(x, y);
                    let ctrl2 = Vec2::from_grid(x, y);
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                    grid.set_used(x, y);
                }
            }
        }
    }
}

// ============================================================================
// Underscore line finding
// ============================================================================

fn find_underscore_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            if grid.get(x, y) == '_' {
                let start_x = x;
                while x < grid.width as i32 && grid.get(x, y) == '_' {
                    grid.set_used(x, y);
                    x += 1;
                }
                let end_x = x - 1;

                if end_x > start_x {
                    // Underscore is at the bottom of the cell
                    let start = Vec2::from_grid(start_x, y).offset(0.0, 0.5);
                    let end = Vec2::from_grid(end_x, y).offset(0.0, 0.5);
                    let path = Path::line(start, end);
                    paths.insert(path);
                }
            } else {
                x += 1;
            }
        }
    }
}

// ============================================================================
// Arrow head finding
// ============================================================================

fn find_arrow_heads(grid: &mut Grid, paths: &PathSet, decorations: &mut DecorationSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);

            match c {
                '>' => {
                    // Right arrow - check for horizontal line to the left
                    if paths.left_ends_at(x, y) || paths.horizontal_passes_through(x - 1, y) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_RIGHT));
                        grid.set_used(x, y);
                    }
                    // Check for diagonal
                    else if paths.diagonal_up_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, arrow_angle_diagonal_up()));
                        grid.set_used(x, y);
                    } else if paths.back_diagonal_down_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, arrow_angle_back_diagonal_down()));
                        grid.set_used(x, y);
                    }
                }
                '<' => {
                    // Left arrow
                    if paths.right_ends_at(x, y) || paths.horizontal_passes_through(x + 1, y) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_LEFT));
                        grid.set_used(x, y);
                    }
                    // Check for diagonal
                    else if paths.diagonal_down_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, arrow_angle_diagonal_down() + 180.0));
                        grid.set_used(x, y);
                    } else if paths.back_diagonal_up_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, arrow_angle_back_diagonal_up() + 180.0));
                        grid.set_used(x, y);
                    }
                }
                '^' => {
                    // Up arrow
                    if paths.down_ends_at(x, y) || paths.vertical_passes_through(x, y + 1) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_UP));
                        grid.set_used(x, y);
                    }
                }
                'v' | 'V' => {
                    // Down arrow
                    if paths.up_ends_at(x, y) || paths.vertical_passes_through(x, y - 1) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_DOWN));
                        grid.set_used(x, y);
                    }
                }
                _ => {}
            }
        }
    }
}

// ============================================================================
// Point decoration finding
// ============================================================================

fn find_points(grid: &mut Grid, _paths: &PathSet, decorations: &mut DecorationSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);

            // Check if this point is adjacent to a line character
            let adjacent_to_line = is_solid_h_line(grid.get(x - 1, y))
                || is_solid_h_line(grid.get(x + 1, y))
                || is_solid_v_line(grid.get(x, y - 1))
                || is_solid_v_line(grid.get(x, y + 1))
                || is_solid_d_line(grid.get(x - 1, y + 1))
                || is_solid_d_line(grid.get(x + 1, y - 1))
                || is_solid_b_line(grid.get(x - 1, y - 1))
                || is_solid_b_line(grid.get(x + 1, y + 1));

            match c {
                '*' => {
                    if adjacent_to_line {
                        decorations.insert(Decoration::closed_point(x, y));
                        grid.set_used(x, y);
                    }
                }
                'o' => {
                    if adjacent_to_line {
                        decorations.insert(Decoration::open_point(x, y));
                        grid.set_used(x, y);
                    }
                }
                '◌' => {
                    decorations.insert(Decoration::dotted_point(x, y));
                    grid.set_used(x, y);
                }
                '○' => {
                    decorations.insert(Decoration::open_point(x, y));
                    grid.set_used(x, y);
                }
                '◍' => {
                    decorations.insert(Decoration::shaded_point(x, y));
                    grid.set_used(x, y);
                }
                '●' => {
                    decorations.insert(Decoration::closed_point(x, y));
                    grid.set_used(x, y);
                }
                '⊕' => {
                    decorations.insert(Decoration::xor_point(x, y));
                    grid.set_used(x, y);
                }
                _ => {}
            }
        }
    }
}

// ============================================================================
// Jump (bridge) finding
// ============================================================================

fn find_jumps(grid: &mut Grid, paths: &PathSet, decorations: &mut DecorationSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);

            // Jump is a ( or ) that bridges a horizontal line crossing
            if c == '(' || c == ')' {
                // Check if there's a vertical line passing through
                if paths.vertical_passes_through(x, y) {
                    let from = Vec2::from_grid(x, y).offset(0.0, -0.5);
                    let to = Vec2::from_grid(x, y).offset(0.0, 0.5);
                    decorations.insert(Decoration::jump(x, y, from, to));
                    grid.set_used(x, y);
                }
            }
        }
    }
}

// ============================================================================
// Gray fill finding
// ============================================================================

fn find_gray_fills(grid: &mut Grid, decorations: &mut DecorationSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);
            if is_gray(c) {
                decorations.insert(Decoration::gray(x, y, c));
                grid.set_used(x, y);
            }
        }
    }
}

// ============================================================================
// Triangle finding
// ============================================================================

fn find_triangles(grid: &mut Grid, decorations: &mut DecorationSet) {
    let width = grid.width as i32;
    let height = grid.height as i32;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);
            if is_tri(c) {
                decorations.insert(Decoration::triangle(x, y, c));
                grid.set_used(x, y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_horizontal_line() {
        let mut grid = Grid::new("---");
        let mut paths = PathSet::new();
        find_paths(&mut grid, &mut paths);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_find_vertical_line() {
        let mut grid = Grid::new("|\n|\n|");
        let mut paths = PathSet::new();
        find_paths(&mut grid, &mut paths);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn test_find_box() {
        let mut grid = Grid::new("+--+\n|  |\n+--+");
        let mut paths = PathSet::new();
        find_paths(&mut grid, &mut paths);
        // Should find 2 horizontal lines and 2 vertical lines
        assert!(paths.len() >= 4);
    }

    #[test]
    fn test_find_arrow() {
        let mut grid = Grid::new("-->");
        let mut paths = PathSet::new();
        let mut decorations = DecorationSet::new();
        find_paths(&mut grid, &mut paths);
        find_decorations(&mut grid, &paths, &mut decorations);
        assert_eq!(decorations.len(), 1);
    }

    #[test]
    fn test_find_diagonal() {
        let mut grid = Grid::new("\\\n \\");
        let mut paths = PathSet::new();
        find_paths(&mut grid, &mut paths);
        assert!(paths.len() >= 1);
    }
}
