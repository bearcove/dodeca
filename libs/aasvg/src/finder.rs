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

/// Check if a vertical line character at (x,y) is part of a vertical line
/// Following JS logic: connects to another line char, vertex, point, arrow, or underscore
fn is_solid_v_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);
    if !is_solid_v_line(c) {
        return false;
    }

    let up = grid.get(x, y - 1);
    let dn = grid.get(x, y + 1);
    let uprt = grid.get(x + 1, y - 1);
    let uplt = grid.get(x - 1, y - 1);

    // Check connections above and below
    is_top_vertex_or_decoration(up)
        || is_solid_v_line(up)
        || is_jump(up)
        || is_bottom_vertex(dn)
        || dn == 'v'
        || dn == 'V'
        || is_solid_v_line(dn)
        || is_jump(dn)
        || is_point(up)
        || is_point(dn)
        || up == '_'
        || uplt == '_'
        || uprt == '_'
        // Special case: 1-high vertical on two curved corners
        || ((is_top_vertex(uplt) || is_top_vertex(uprt))
            && (is_bottom_vertex(grid.get(x - 1, y + 1)) || is_bottom_vertex(grid.get(x + 1, y + 1))))
}

fn find_solid_vertical_lines(grid: &mut Grid, paths: &mut PathSet) {
    for x in 0..grid.width as i32 {
        let mut y = 0;
        while y < grid.height as i32 {
            if is_solid_v_line_at(grid, x, y) && !grid.is_used(x, y) {
                let start_y = y;
                while y < grid.height as i32 && is_solid_v_line_at(grid, x, y) {
                    grid.set_used(x, y);
                    y += 1;
                }
                let end_y = y - 1;

                // Adjust endpoints for vertices and mark them as used
                let mut adj_start_y = start_y;
                let mut adj_end_y = end_y;

                // Check if we should extend to connect with vertices
                if is_top_vertex(grid.get(x, start_y - 1)) {
                    adj_start_y = start_y - 1;
                    grid.set_used(x, adj_start_y);
                }
                if is_bottom_vertex(grid.get(x, end_y + 1)) {
                    adj_end_y = end_y + 1;
                    grid.set_used(x, adj_end_y);
                }

                let path = Path::line_from_grid(x, adj_start_y, x, adj_end_y);
                paths.insert(path);
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

/// Check if position is part of a solid horizontal line
/// Following JS logic: "We need three in a row"
/// A position is part of a horizontal line if:
/// - It's a solid horizontal line character (-) with proper continuation, OR
/// - It's a vertex with at least 2 horizontal line chars on one side
fn is_solid_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y);
    let ltlt = grid.get(x - 2, y);
    let rt = grid.get(x + 1, y);
    let rtrt = grid.get(x + 2, y);

    if is_solid_h_line(c) {
        // Need three in a row (including vertices at ends)
        if is_solid_h_line(lt) {
            // Has line char to left - need line or vertex to right, or line to far left
            return is_solid_h_line(rt)
                || is_vertex_or_right_decoration(rt)
                || is_solid_h_line(ltlt)
                || is_vertex_or_left_decoration(ltlt);
        } else if is_vertex_or_left_decoration(lt) {
            // Vertex to left - need line char to right
            return is_solid_h_line(rt);
        } else {
            // Need line to right AND (line or vertex at far right)
            return is_solid_h_line(rt)
                && (is_solid_h_line(rtrt) || is_vertex_or_right_decoration(rtrt));
        }
    } else if is_vertex(c) {
        // Vertex is part of line if there are 2 line chars on one side
        (is_solid_h_line(lt) && is_solid_h_line(ltlt))
            || (is_solid_h_line(rt) && is_solid_h_line(rtrt))
    } else {
        false
    }
}

fn find_solid_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            if is_solid_h_line_at(grid, x, y) && !grid.is_used(x, y) {
                let start_x = x;
                while x < grid.width as i32 && is_solid_h_line_at(grid, x, y) {
                    grid.set_used(x, y);
                    x += 1;
                }
                let end_x = x - 1;

                if end_x > start_x {
                    let path = Path::line_from_grid(start_x, y, end_x, y);
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

/// Check if position is part of a double horizontal line
/// Similar to solid horizontal line logic but for = characters
fn is_double_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y);
    let ltlt = grid.get(x - 2, y);
    let rt = grid.get(x + 1, y);
    let rtrt = grid.get(x + 2, y);

    if is_double_h_line(c) && c != '+' && c != '(' && c != ')' {
        // Need three in a row (including vertices at ends)
        if is_double_h_line(lt) {
            return is_double_h_line(rt)
                || is_vertex_or_right_decoration(rt)
                || is_double_h_line(ltlt)
                || is_vertex_or_left_decoration(ltlt);
        } else if is_vertex_or_left_decoration(lt) {
            return is_double_h_line(rt);
        } else {
            return is_double_h_line(rt)
                && (is_double_h_line(rtrt) || is_vertex_or_right_decoration(rtrt));
        }
    } else if is_vertex(c) {
        // Vertex is part of line if there are 2 line chars on one side
        (is_double_h_line(lt) && is_double_h_line(ltlt))
            || (is_double_h_line(rt) && is_double_h_line(rtrt))
    } else {
        false
    }
}

fn find_double_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            if is_double_h_line_at(grid, x, y) && !grid.is_used(x, y) {
                let start_x = x;
                while x < grid.width as i32 && is_double_h_line_at(grid, x, y) {
                    grid.set_used(x, y);
                    x += 1;
                }
                let end_x = x - 1;

                if end_x > start_x {
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

/// Check if a backslash at (x,y) is part of a diagonal line
/// Following JS logic: connects to another diagonal, vertex, point, arrow, or underscore
fn is_solid_b_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y - 1); // upper-left
    let rt = grid.get(x + 1, y + 1); // lower-right

    if is_solid_b_line(c) {
        // Check connections
        is_solid_b_line(rt)
            || is_bottom_vertex(rt)
            || is_point(rt)
            || rt == 'v'
            || rt == 'V'
            || is_solid_b_line(lt)
            || is_top_vertex(lt)
            || is_point(lt)
            || lt == '^'
            || grid.get(x, y - 1) == '/'  // hexagon corner
            || grid.get(x, y + 1) == '/'  // hexagon corner
            || rt == '_'
            || lt == '_'
    } else if is_vertex(c) || is_point(c) || c == '|' {
        // Vertex/point/pipe is part of diagonal if adjacent to backslash
        is_solid_b_line(lt) || is_solid_b_line(rt)
    } else {
        false
    }
}

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
            if is_solid_b_line_at(grid, x, y) && !grid.is_used(x, y) {
                let line_start_x = x;
                let line_start_y = y;

                while x < width && y < height && is_solid_b_line_at(grid, x, y) {
                    grid.set_used(x, y);
                    x += 1;
                    y += 1;
                }

                let line_end_x = x - 1;
                let line_end_y = y - 1;

                // Check if we should extend to vertices at the ends
                let mut adj_start_x = line_start_x;
                let mut adj_start_y = line_start_y;
                let mut adj_end_x = line_end_x;
                let mut adj_end_y = line_end_y;

                // Upper-left end: check for top vertex
                let ul = grid.get(line_start_x - 1, line_start_y - 1);
                if is_top_vertex(ul) || ul == '^' {
                    adj_start_x = line_start_x - 1;
                    adj_start_y = line_start_y - 1;
                    grid.set_used(adj_start_x, adj_start_y);
                }

                // Lower-right end: check for bottom vertex
                let lr = grid.get(line_end_x + 1, line_end_y + 1);
                if is_bottom_vertex(lr) || lr == 'v' || lr == 'V' {
                    adj_end_x = line_end_x + 1;
                    adj_end_y = line_end_y + 1;
                    grid.set_used(adj_end_x, adj_end_y);
                }

                let path = Path::line_from_grid(adj_start_x, adj_start_y, adj_end_x, adj_end_y);
                paths.insert(path);
            } else {
                x += 1;
                y += 1;
            }
        }
    }
}

/// Check if a forward slash at (x,y) is part of a diagonal line
/// Following JS logic: connects to another diagonal, vertex, point, arrow, or underscore
fn is_solid_d_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y + 1); // lower-left
    let rt = grid.get(x + 1, y - 1); // upper-right

    if is_solid_d_line(c) {
        // Special case: hexagon corner with backslash
        if grid.get(x, y - 1) == '\\' || grid.get(x, y + 1) == '\\' {
            return true;
        }

        // Check connections
        is_solid_d_line(rt)
            || is_top_vertex(rt)
            || is_point(rt)
            || rt == '^'
            || rt == '_'
            || is_solid_d_line(lt)
            || is_bottom_vertex(lt)
            || is_point(lt)
            || lt == 'v'
            || lt == 'V'
            || lt == '_'
    } else if is_vertex(c) || is_point(c) || c == '|' {
        // Vertex/point/pipe is part of diagonal if adjacent to forward slash
        is_solid_d_line(lt) || is_solid_d_line(rt)
    } else {
        false
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
            if is_solid_d_line_at(grid, x, y) && !grid.is_used(x, y) {
                let line_start_x = x;
                let line_start_y = y;

                while x >= 0 && y < height && is_solid_d_line_at(grid, x, y) {
                    grid.set_used(x, y);
                    x -= 1;
                    y += 1;
                }

                let line_end_x = x + 1;
                let line_end_y = y - 1;

                // Check if we should extend to vertices at the ends
                let mut adj_start_x = line_start_x;
                let mut adj_start_y = line_start_y;
                let mut adj_end_x = line_end_x;
                let mut adj_end_y = line_end_y;

                // Upper-right end: check for top vertex
                let ur = grid.get(line_start_x + 1, line_start_y - 1);
                if is_top_vertex(ur) || ur == '^' {
                    adj_start_x = line_start_x + 1;
                    adj_start_y = line_start_y - 1;
                    grid.set_used(adj_start_x, adj_start_y);
                }

                // Lower-left end: check for bottom vertex
                let ll = grid.get(line_end_x - 1, line_end_y + 1);
                if is_bottom_vertex(ll) || ll == 'v' || ll == 'V' {
                    adj_end_x = line_end_x - 1;
                    adj_end_y = line_end_y + 1;
                    grid.set_used(adj_end_x, adj_end_y);
                }

                // For forward slash: create path from bottom-left to top-right
                let path = Path::line_from_grid(adj_end_x, adj_end_y, adj_start_x, adj_start_y);
                paths.insert(path);
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

    // Bezier circle approximation constant
    // https://spencermortensen.com/articles/bezier-circle/
    const CURVE: f64 = 0.551915024494;
    const CURVE_X: f64 = 2.0 * CURVE;
    const CURVE_Y: f64 = CURVE;

    for y in 0..height {
        for x in 0..width {
            let c = grid.get(x, y);

            // Top vertex patterns (. or ,)
            if is_top_vertex(c) {
                // -.
                //   |
                // Check for horizontal line to left and vertical line at (x+1, y+1)
                if is_solid_h_line(grid.get(x - 1, y)) && is_solid_v_line(grid.get(x + 1, y + 1)) {
                    grid.set_used(x - 1, y);
                    grid.set_used(x, y);
                    grid.set_used(x + 1, y + 1);
                    let start = Vec2::from_grid(x - 1, y);
                    let end = Vec2::from_grid(x + 1, y + 1);
                    let ctrl1 = Vec2::new(
                        start.x + CURVE_X * crate::path::SCALE,
                        start.y,
                    );
                    let ctrl2 = Vec2::new(
                        end.x,
                        end.y - CURVE_Y * crate::path::SCALE * crate::path::ASPECT,
                    );
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                }

                //  .-
                // |
                // Check for horizontal line to right and vertical line at (x-1, y+1)
                if is_solid_h_line(grid.get(x + 1, y)) && is_solid_v_line(grid.get(x - 1, y + 1)) {
                    grid.set_used(x - 1, y + 1);
                    grid.set_used(x, y);
                    grid.set_used(x + 1, y);
                    let start = Vec2::from_grid(x + 1, y);
                    let end = Vec2::from_grid(x - 1, y + 1);
                    let ctrl1 = Vec2::new(
                        start.x - CURVE_X * crate::path::SCALE,
                        start.y,
                    );
                    let ctrl2 = Vec2::new(
                        end.x,
                        end.y - CURVE_Y * crate::path::SCALE * crate::path::ASPECT,
                    );
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                }
            }

            // Special case patterns for round boxes:
            //   .  .   .  .
            //  (  o     )  o
            //   '  .   '  '
            if (c == ')' || is_point(c))
                && grid.get(x - 1, y - 1) == '.'
                && grid.get(x - 1, y + 1) == '\''
            {
                grid.set_used(x, y);
                grid.set_used(x - 1, y - 1);
                grid.set_used(x - 1, y + 1);
                let start = Vec2::from_grid(x - 2, y - 1);
                let end = Vec2::from_grid(x - 2, y + 1);
                let ctrl1 = Vec2::new(
                    (x as f64 + 0.6) * crate::path::SCALE,
                    start.y,
                );
                let ctrl2 = Vec2::new(
                    (x as f64 + 0.6) * crate::path::SCALE,
                    end.y,
                );
                let path = Path::curve(start, end, ctrl1, ctrl2);
                paths.insert(path);
            }

            if (c == '(' || is_point(c))
                && grid.get(x + 1, y - 1) == '.'
                && grid.get(x + 1, y + 1) == '\''
            {
                grid.set_used(x, y);
                grid.set_used(x + 1, y - 1);
                grid.set_used(x + 1, y + 1);
                let start = Vec2::from_grid(x + 2, y - 1);
                let end = Vec2::from_grid(x + 2, y + 1);
                let ctrl1 = Vec2::new(
                    (x as f64 - 0.6) * crate::path::SCALE,
                    start.y,
                );
                let ctrl2 = Vec2::new(
                    (x as f64 - 0.6) * crate::path::SCALE,
                    end.y,
                );
                let path = Path::curve(start, end, ctrl1, ctrl2);
                paths.insert(path);
            }

            // Bottom vertex patterns (' or `)
            if is_bottom_vertex(c) {
                //   |
                // -'
                // Check for horizontal line to left and vertical line at (x+1, y-1)
                if is_solid_h_line(grid.get(x - 1, y)) && is_solid_v_line(grid.get(x + 1, y - 1)) {
                    grid.set_used(x - 1, y);
                    grid.set_used(x, y);
                    grid.set_used(x + 1, y - 1);
                    let start = Vec2::from_grid(x - 1, y);
                    let end = Vec2::from_grid(x + 1, y - 1);
                    let ctrl1 = Vec2::new(
                        start.x + CURVE_X * crate::path::SCALE,
                        start.y,
                    );
                    let ctrl2 = Vec2::new(
                        end.x,
                        end.y + CURVE_Y * crate::path::SCALE * crate::path::ASPECT,
                    );
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
                }

                // |
                //  '-
                // Check for horizontal line to right and vertical line at (x-1, y-1)
                if is_solid_h_line(grid.get(x + 1, y)) && is_solid_v_line(grid.get(x - 1, y - 1)) {
                    grid.set_used(x - 1, y - 1);
                    grid.set_used(x, y);
                    grid.set_used(x + 1, y);
                    let start = Vec2::from_grid(x + 1, y);
                    let end = Vec2::from_grid(x - 1, y - 1);
                    let ctrl1 = Vec2::new(
                        start.x - CURVE_X * crate::path::SCALE,
                        start.y,
                    );
                    let ctrl2 = Vec2::new(
                        end.x,
                        end.y + CURVE_Y * crate::path::SCALE * crate::path::ASPECT,
                    );
                    let path = Path::curve(start, end, ctrl1, ctrl2);
                    paths.insert(path);
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
                    // Up arrow - check for vertical line below or solid line char directly below
                    if paths.down_ends_at(x, y)
                        || paths.vertical_passes_through(x, y + 1)
                        || is_solid_v_line(grid.get(x, y + 1))
                        || is_double_v_line(grid.get(x, y + 1))
                    {
                        decorations.insert(Decoration::arrow(x, y, ARROW_UP));
                        grid.set_used(x, y);
                    }
                }
                'v' | 'V' => {
                    // Down arrow - check for vertical line above or solid line char directly above
                    if paths.up_ends_at(x, y)
                        || paths.vertical_passes_through(x, y - 1)
                        || is_solid_v_line(grid.get(x, y - 1))
                        || is_double_v_line(grid.get(x, y - 1))
                    {
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

/// Check if a character is "empty or vertex" for point detection
/// Matches: space, or any non-alphanumeric, or 'o'/'v'
fn is_empty_or_vertex(c: char) -> bool {
    c == ' ' || !c.is_ascii_alphanumeric() || c == 'o' || c == 'v'
}

/// Check if the point is on a line (surrounded by non-text)
fn on_line(grid: &Grid, x: i32, y: i32) -> bool {
    let up = grid.get(x, y - 1);
    let dn = grid.get(x, y + 1);
    let lt = grid.get(x - 1, y);
    let rt = grid.get(x + 1, y);

    (is_empty_or_vertex(dn) || is_point(dn))
        && (is_empty_or_vertex(up) || is_point(up))
        && is_empty_or_vertex(rt)
        && is_empty_or_vertex(lt)
}

fn find_points(grid: &mut Grid, paths: &PathSet, decorations: &mut DecorationSet) {
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

            // Also check if a path ends at this point
            let path_ends_here = paths.right_ends_at(x - 1, y)
                || paths.left_ends_at(x + 1, y)
                || paths.down_ends_at(x, y - 1)
                || paths.up_ends_at(x, y + 1)
                || paths.up_ends_at(x, y)
                || paths.down_ends_at(x, y);

            // Or if it's surrounded by non-text (on a line)
            let is_on_line = on_line(grid, x, y);

            match c {
                '*' => {
                    if adjacent_to_line || path_ends_here || is_on_line {
                        decorations.insert(Decoration::closed_point(x, y));
                        grid.set_used(x, y);
                    }
                }
                'o' => {
                    if adjacent_to_line || path_ends_here || is_on_line {
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
