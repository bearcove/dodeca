//! Path and decoration finding algorithms.
//!
//! This module scans the grid to find all lines, curves, and decorations.

use crate::chars::*;
use crate::decoration::*;
use crate::grid::Grid;
use crate::path::*;

/// Find all paths (lines and curves) in the grid
pub fn find_paths(grid: &mut Grid, paths: &mut PathSet) {
    find_vertical_lines(grid, paths); // Combined solid and double, interleaved like JS
    find_circuit_diagram_short_lines(grid, paths); // Must come after vline finders
    find_horizontal_lines(grid, paths); // Combined solid, squiggle, and double, interleaved like JS
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
/// Following JS logic from isVLineAt in markdeep-diagram.js lines 321-360
fn is_solid_v_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);
    let up = grid.get(x, y - 1);
    let dn = grid.get(x, y + 1);
    let uprt = grid.get(x + 1, y - 1);
    let uplt = grid.get(x - 1, y - 1);

    if is_solid_v_line(c) {
        // Looks like a vertical line...does it continue?
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
    } else if is_top_vertex(c) || c == '^' {
        // May be the top of a vertical line
        is_solid_v_line(dn) || (is_jump(dn) && c != '.')
    } else if is_bottom_vertex(c) || c == 'v' || c == 'V' {
        // May be the bottom of a vertical line
        is_solid_v_line(up) || (is_jump(up) && c != '\'')
    } else if is_point(c) {
        is_solid_v_line(up) || is_solid_v_line(dn)
    } else {
        false
    }
}

/// Check if a double vertical line character at (x,y) is part of a vertical line
/// Following JS logic from isVLineAt in markdeep-diagram.js lines 321-360
/// Called as grid.isDoubleVLineAt(x, y)
fn is_double_v_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);
    let up = grid.get(x, y - 1);
    let dn = grid.get(x, y + 1);
    let uprt = grid.get(x + 1, y - 1);
    let uplt = grid.get(x - 1, y - 1);

    if is_double_v_line(c) {
        // Looks like a vertical line...does it continue?
        is_top_vertex_or_decoration(up)
            || is_double_v_line(up)
            || is_jump(up)
            || is_bottom_vertex(dn)
            || dn == 'v'
            || dn == 'V'
            || is_double_v_line(dn)
            || is_jump(dn)
            || is_point(up)
            || is_point(dn)
            || up == '_'
            || uplt == '_'
            || uprt == '_'
            // Special case: 1-high vertical on two curved corners
            || ((is_top_vertex(uplt) || is_top_vertex(uprt))
                && (is_bottom_vertex(grid.get(x - 1, y + 1)) || is_bottom_vertex(grid.get(x + 1, y + 1))))
    } else if is_top_vertex(c) || c == '^' {
        // May be the top of a vertical line
        is_double_v_line(dn) || (is_jump(dn) && c != '.')
    } else if is_bottom_vertex(c) || c == 'v' || c == 'V' {
        // May be the bottom of a vertical line
        is_double_v_line(up) || (is_jump(up) && c != '\'')
    } else if is_point(c) {
        is_double_v_line(up) || is_double_v_line(dn)
    } else {
        false
    }
}

/// Stretch vertical line endpoints to meet adjacent lines and decorations
/// Returns (adjusted_start_y, adjusted_end_y) as fractional grid coordinates
/// This implements the JS stretching logic from markdeep-diagram.js lines 847-864
/// `is_double` indicates if this is a double line (affects which alt check and box chars to use)
fn stretch_v_line_endpoints(
    grid: &Grid,
    x: i32,
    start_y: i32,
    end_y: i32,
    is_double: bool,
) -> (f64, f64) {
    let mut a = start_y as f64;
    let mut b = end_y as f64;

    let up = grid.get(x, start_y);
    let upup = grid.get(x, start_y - 1);
    let dn = grid.get(x, end_y);
    let dndn = grid.get(x, end_y + 1);

    // Box-drawing characters for vertical lines (from JS boxt/boxb params)
    // Solid: boxt = '\u2564' (╤), boxb = '\u2567' (╧)
    // Double: boxt = '\u2565\u2566' (╥╦), boxb = '\u2568\u2569' (╨╩)
    let is_box_top = if is_double {
        |c: char| "╥╦".contains(c)
    } else {
        |c: char| "╤".contains(c)
    };
    let is_box_bottom = if is_double {
        |c: char| "╨╩".contains(c)
    } else {
        |c: char| "╧".contains(c)
    };

    // Check for underscore to left or right of start
    let has_underscore_above =
        grid.get(x - 1, start_y - 1) == '_' || grid.get(x + 1, start_y - 1) == '_';
    // Check for underscore to left or right of end
    let has_underscore_at_end = grid.get(x - 1, end_y) == '_' || grid.get(x + 1, end_y) == '_';

    // Check for alternate line type at position
    // For solid lines: alt = isDoubleVLineAt
    // For double lines: alt = isSolidVLineAt
    let alt_above = if is_double {
        is_solid_v_line_at(grid, x, start_y - 1)
    } else {
        is_double_v_line_at(grid, x, start_y - 1)
    };
    let alt_below = if is_double {
        is_solid_v_line_at(grid, x, end_y + 1)
    } else {
        is_double_v_line_at(grid, x, end_y + 1)
    };

    // === TOP (A) ADJUSTMENTS ===
    // JS: if (!isVertex(up) && ((upup === '-') || (upup === '_') || (boxt.indexOf(upup) >= 0) ||
    //         (grid(A.x - 1, A.y - 1) === '_') || (grid(A.x + 1, A.y - 1) === '_') ||
    //         isBottomVertex(upup)) || isJump(upup) ||
    //         (grid[alt](A.x, A.y - 1) && !isVertex(upup)))
    let stretch_up = (!is_vertex(up)
        && (upup == '-'
            || upup == '_'
            || is_box_top(upup)
            || has_underscore_above
            || is_bottom_vertex(upup)))
        || is_jump(upup)
        || (alt_above && !is_vertex(upup));

    if stretch_up {
        // JS: A.y -= (isVertex(upup) && grid[alt](A.x, A.y - 2) && !isTopVertexOrDecoration(up)) ? 1 : 0.5;
        let alt_above_2 = if is_double {
            is_solid_v_line_at(grid, x, start_y - 2)
        } else {
            is_double_v_line_at(grid, x, start_y - 2)
        };
        if is_vertex(upup) && alt_above_2 && !is_top_vertex(up) && up != '^' {
            a -= 1.0;
        } else {
            a -= 0.5;
        }
    }

    // === BOTTOM (B) ADJUSTMENTS ===
    // JS: if (!isVertex(dn) && ((dndn === '-') || (boxb.indexOf(dndn) >= 0) || isTopVertex(dndn)) ||
    //         isJump(dndn) ||
    //         (grid(B.x - 1, B.y) === '_') || (grid(B.x + 1, B.y) === '_') ||
    //         (grid[alt](B.x, B.y + 1) && !isVertex(dn)))
    let stretch_down = (!is_vertex(dn)
        && (dndn == '-' || is_box_bottom(dndn) || is_top_vertex(dndn)))
        || is_jump(dndn)
        || has_underscore_at_end
        || (alt_below && !is_vertex(dn));

    if stretch_down {
        b += 0.5;
    }

    (a, b)
}

/// Find all vertical lines (solid and double), checking both at each position
/// This matches JS behavior where solid is tried first, then double, at each (x, y)
/// See markdeep-diagram.js lines 833-882
fn find_vertical_lines(grid: &mut Grid, paths: &mut PathSet) {
    for x in 0..grid.width as i32 {
        let mut y = 0;
        while y < grid.height as i32 {
            // Try solid first, then double (matching JS order)
            if try_vline(grid, paths, x, &mut y, false) || try_vline(grid, paths, x, &mut y, true) {
                // Line was found and processed, continue to next position
                // (y was already updated by try_vline)
                continue;
            }
            y += 1;
        }
    }
}

/// Try to find and process a vertical line at (x, y)
/// Returns true if a line was found, false otherwise
/// If a line is found, y is updated to point to the position after the line
/// `is_double` controls whether we're looking for solid (false) or double (true) lines
fn try_vline(grid: &mut Grid, paths: &mut PathSet, x: i32, y: &mut i32, is_double: bool) -> bool {
    let check_fn = if is_double {
        is_double_v_line_at
    } else {
        is_solid_v_line_at
    };

    if !check_fn(grid, x, *y) || grid.is_used(x, *y) {
        return false;
    }

    // This character begins a vertical line...find the end
    let start_y = *y;

    // Mark cells as used and advance y while the line continues
    loop {
        grid.set_used(x, *y);
        *y += 1;
        if *y >= grid.height as i32 || !check_fn(grid, x, *y) {
            break;
        }
    }

    let end_y = *y - 1;

    // Apply stretching logic
    let (adj_start_y, adj_end_y) = stretch_v_line_endpoints(grid, x, start_y, end_y, is_double);

    // Don't insert degenerate lines (JS: if ((A.x !== B.x) || (A.y !== B.y)))
    if adj_start_y != adj_end_y {
        let mut path = Path::line(
            Vec2::from_grid_frac(x as f64, adj_start_y),
            Vec2::from_grid_frac(x as f64, adj_end_y),
        );
        if is_double {
            path = path.with_double(true);
        }
        paths.insert(path);
    }

    // Whether degenerate or not, we advanced y past the line, so return true
    true
}

/// Find special short vertical lines for circuit diagrams
/// JS markdeep-diagram.js lines 886-916
fn find_circuit_diagram_short_lines(grid: &mut Grid, paths: &mut PathSet) {
    for x in 0..grid.width as i32 {
        for y in 0..grid.height as i32 {
            let c = grid.get(x, y);

            // Pattern:    _  _
            //           -'    '-   -'
            // Creates a short vertical line from (x, y-0.5) to (x, y)
            if c == '\'' {
                let lt = grid.get(x - 1, y);
                let rt = grid.get(x + 1, y);
                let uplt = grid.get(x - 1, y - 1);
                let uprt = grid.get(x + 1, y - 1);

                if (lt == '-' && uprt == '_' && !is_solid_v_line_or_jump_or_point(uplt))
                    || (uplt == '_' && rt == '-' && !is_solid_v_line_or_jump_or_point(uprt))
                {
                    paths.insert(Path::line(
                        Vec2::from_grid_frac(x as f64, y as f64 - 0.5),
                        Vec2::from_grid(x, y),
                    ));
                }
            }
            // Pattern: _.-  -._
            // Creates a short vertical line from (x, y) to (x, y+0.5)
            else if c == '.' {
                let lt = grid.get(x - 1, y);
                let rt = grid.get(x + 1, y);
                let dnlt = grid.get(x - 1, y + 1);
                let dnrt = grid.get(x + 1, y + 1);

                if (lt == '_' && rt == '-' && !is_solid_v_line_or_jump_or_point(dnrt))
                    || (lt == '-' && rt == '_' && !is_solid_v_line_or_jump_or_point(dnlt))
                {
                    paths.insert(Path::line(
                        Vec2::from_grid(x, y),
                        Vec2::from_grid_frac(x as f64, y as f64 + 0.5),
                    ));
                }
                // For drawing resistors: -.╱
                else if lt == '-' && rt == '\u{2571}' {
                    paths.insert(Path::line(
                        Vec2::from_grid(x, y),
                        Vec2::from_grid_frac(x as f64 + 0.5, y as f64 + 0.5),
                    ));
                }
            }
            // For drawing resistors: ╱'-
            else if c == '\'' && grid.get(x + 1, y) == '-' && grid.get(x - 1, y) == '\u{2571}' {
                paths.insert(Path::line(
                    Vec2::from_grid(x, y),
                    Vec2::from_grid_frac(x as f64 - 0.5, y as f64 - 0.5),
                ));
            }
        }
    }
}

// ============================================================================
// Horizontal line finding
// ============================================================================

/// Stretch horizontal line endpoints to meet adjacent lines of different types
/// Returns (adjusted_start, adjusted_end) as fractional grid coordinates
/// This implements the full JS stretching logic from markdeep-diagram.js lines 933-962
fn stretch_h_line_endpoints(grid: &Grid, start_x: i32, end_x: i32, y: i32) -> (f64, f64) {
    let mut a = start_x as f64;
    let mut b = end_x as f64;

    let start_char = grid.get(start_x, y);
    let end_char = grid.get(end_x, y);
    let left_of_start = grid.get(start_x - 1, y);
    let right_of_end = grid.get(end_x + 1, y);

    // Box-drawing characters that extend solid horizontal lines
    // JS: for solid/squiggle lines: boxl = '\u2523' (┣), boxr = '\u252b' (┫)
    let is_box_left = |c: char| c == '┣';
    let is_box_right = |c: char| c == '┫';

    // === INDEPENDENT BOX-DRAWING EXTENSIONS (JS lines 933-934) ===
    // These are checked first and independently of the else-if chains below
    if is_box_right(right_of_end) {
        b += 0.5;
    }
    if is_box_left(left_of_start) {
        a -= 0.5;
    }

    // === LEFT SIDE (A) ADJUSTMENTS (JS lines 937-949) ===
    // This is an else-if chain
    // 1. Curve detection and shortening - if at a curve vertex, shorten by 1
    if !is_vertex(left_of_start)
        && ((is_top_vertex(start_char)
            && is_solid_v_line_or_jump_or_point(grid.get(start_x - 1, y + 1)))
            || (is_bottom_vertex(start_char)
                && is_solid_v_line_or_jump_or_point(grid.get(start_x - 1, y - 1))))
    {
        a += 1.0;
    }
    // 2. Line continuation - if there's a vertex to the left with a line continuing
    else if !is_vertex_or_left_decoration(start_char)
        && is_vertex(left_of_start)
        && is_any_h_line_at(grid, start_x - 2, y)
    {
        a -= 1.0;
    }
    // 3. Adjacent line stretching - extend by 0.5 to meet adjacent line type
    else if is_any_h_line_at(grid, start_x - 1, y)
        && !is_vertex_or_right_decoration(left_of_start)
        && !is_vertex(start_char)
    {
        a -= 0.5;
    }

    // === RIGHT SIDE (B) ADJUSTMENTS (JS lines 951-957) ===
    // This is an else-if chain
    // 1. Curve detection and shortening
    if !is_vertex(right_of_end)
        && ((is_top_vertex(end_char)
            && is_solid_v_line_or_jump_or_point(grid.get(end_x + 1, y + 1)))
            || (is_bottom_vertex(end_char)
                && is_solid_v_line_or_jump_or_point(grid.get(end_x + 1, y - 1))))
    {
        b -= 1.0;
    }
    // 2. Adjacent line stretching
    else if is_any_h_line_at(grid, end_x + 1, y)
        && !is_vertex_or_left_decoration(right_of_end)
        && !is_vertex(end_char)
    {
        b += 0.5;
    }

    (a, b)
}

/// Check if character is a solid vertical line, jump, or point
fn is_solid_v_line_or_jump_or_point(c: char) -> bool {
    is_solid_v_line(c) || is_jump(c) || is_point(c)
}

/// Check if any horizontal line (solid, squiggle, or double) is at the position
fn is_any_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    is_solid_h_line_at(grid, x, y)
        || is_squiggle_h_line_at(grid, x, y)
        || is_double_h_line_at(grid, x, y)
}

/// Check if position is part of a solid horizontal line
/// Following JS logic from isHLineAt in markdeep-diagram.js lines 362-389
fn is_solid_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y);
    let ltlt = grid.get(x - 2, y);
    let rt = grid.get(x + 1, y);
    let rtrt = grid.get(x + 2, y);

    // JS: if (f(c) || (f(lt) && isJump(c)))
    if is_solid_h_line(c) || (is_solid_h_line(lt) && is_jump(c)) {
        // Looks like a horizontal line...does it continue? We need three in a row.
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
    } else if c == '<' {
        // Left arrow is part of solid line if there are two line chars to the right
        is_solid_h_line(rt) && is_solid_h_line(rtrt)
    } else if c == '>' {
        // Right arrow is part of solid line if there are two line chars to the left
        is_solid_h_line(lt) && is_solid_h_line(ltlt)
    } else if is_vertex(c) {
        // Vertex is part of line if there are 2 line chars on one side
        (is_solid_h_line(lt) && is_solid_h_line(ltlt))
            || (is_solid_h_line(rt) && is_solid_h_line(rtrt))
    } else {
        false
    }
}

/// Find all horizontal lines (solid, squiggle, and double), checking all at each position
/// This matches JS behavior where solid is tried first, then squiggle, then double
/// See markdeep-diagram.js lines 924-979
fn find_horizontal_lines(grid: &mut Grid, paths: &mut PathSet) {
    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < grid.width as i32 {
            // Try solid first, then squiggle, then double (matching JS order)
            if try_hline(grid, paths, &mut x, y, HLineType::Solid)
                || try_hline(grid, paths, &mut x, y, HLineType::Squiggle)
                || try_hline(grid, paths, &mut x, y, HLineType::Double)
            {
                // Line was found and processed, continue to next position
                continue;
            }
            x += 1;
        }
    }
}

#[derive(Clone, Copy)]
enum HLineType {
    Solid,
    Squiggle,
    Double,
}

/// Try to find and process a horizontal line at (x, y)
/// Returns true if a line was found, false otherwise
fn try_hline(
    grid: &mut Grid,
    paths: &mut PathSet,
    x: &mut i32,
    y: i32,
    line_type: HLineType,
) -> bool {
    let check_fn = match line_type {
        HLineType::Solid => is_solid_h_line_at,
        HLineType::Squiggle => is_squiggle_h_line_at,
        HLineType::Double => is_double_h_line_at,
    };

    if !check_fn(grid, *x, y) {
        return false;
    }

    // This character begins a horizontal line...find the end
    let start_x = *x;

    // Mark cells as used and advance x while the line continues
    loop {
        grid.set_used(*x, y);
        *x += 1;
        if *x >= grid.width as i32 || !check_fn(grid, *x, y) {
            break;
        }
    }

    let end_x = *x - 1;

    // Apply stretching logic
    let (adj_start, adj_end) = stretch_h_line_endpoints(grid, start_x, end_x, y);

    // Only insert non-degenerate lines (JS: if ((A.x !== B.x) || (A.y !== B.y)))
    if adj_start != adj_end {
        let mut path = Path::line(
            Vec2::from_grid_frac(adj_start, y as f64),
            Vec2::from_grid_frac(adj_end, y as f64),
        );
        match line_type {
            HLineType::Solid => {}
            HLineType::Squiggle => path = path.with_squiggle(true),
            HLineType::Double => path = path.with_double(true),
        }
        paths.insert(path);
        return true;
    }

    // Degenerate line (after stretching) - we already advanced x past the line
    true
}

/// Check if position is part of a squiggle horizontal line
/// Following JS logic from isHLineAt
fn is_squiggle_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y);
    let ltlt = grid.get(x - 2, y);
    let rt = grid.get(x + 1, y);
    let rtrt = grid.get(x + 2, y);

    // JS: if (f(c) || (f(lt) && isJump(c)))
    if (is_squiggle_h_line(c) && c != '+') || (is_squiggle_h_line(lt) && is_jump(c)) {
        // Looks like a horizontal line...does it continue? We need three in a row.
        if is_squiggle_h_line(lt) {
            return is_squiggle_h_line(rt)
                || is_vertex_or_right_decoration(rt)
                || is_squiggle_h_line(ltlt)
                || is_vertex_or_left_decoration(ltlt);
        } else if is_vertex_or_left_decoration(lt) {
            return is_squiggle_h_line(rt);
        } else {
            return is_squiggle_h_line(rt)
                && (is_squiggle_h_line(rtrt) || is_vertex_or_right_decoration(rtrt));
        }
    } else if c == '<' {
        is_squiggle_h_line(rt) && is_squiggle_h_line(rtrt)
    } else if c == '>' {
        is_squiggle_h_line(lt) && is_squiggle_h_line(ltlt)
    } else if is_vertex(c) {
        (is_squiggle_h_line(lt) && is_squiggle_h_line(ltlt))
            || (is_squiggle_h_line(rt) && is_squiggle_h_line(rtrt))
    } else {
        false
    }
}

/// Check if position is part of a double horizontal line
/// Following JS logic from isHLineAt
fn is_double_h_line_at(grid: &Grid, x: i32, y: i32) -> bool {
    let c = grid.get(x, y);

    let lt = grid.get(x - 1, y);
    let ltlt = grid.get(x - 2, y);
    let rt = grid.get(x + 1, y);
    let rtrt = grid.get(x + 2, y);

    // JS: if (f(c) || (f(lt) && isJump(c)))
    if (is_double_h_line(c) && c != '+' && c != '(' && c != ')')
        || (is_double_h_line(lt) && is_jump(c))
    {
        // Looks like a horizontal line...does it continue? We need three in a row.
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
    } else if c == '<' {
        is_double_h_line(rt) && is_double_h_line(rtrt)
    } else if c == '>' {
        is_double_h_line(lt) && is_double_h_line(ltlt)
    } else if is_vertex(c) {
        (is_double_h_line(lt) && is_double_h_line(ltlt))
            || (is_double_h_line(rt) && is_double_h_line(rtrt))
    } else {
        false
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
    // JS: markdeep-diagram.js lines 983-1055
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
            // Note: JS doesn't check isUsed before starting a diagonal
            // This allows vertices to be shared between backslash and forward-slash diagonals
            if is_solid_b_line_at(grid, x, y) {
                let line_start_x = x;
                let line_start_y = y;

                // Find the end of the line
                while x < width && y < height && is_solid_b_line_at(grid, x, y) {
                    x += 1;
                    y += 1;
                }

                let line_end_x = x - 1;
                let line_end_y = y - 1;

                // Check if the line contains at least one actual backslash
                let mut has_backslash = false;
                for j in line_start_x..=line_end_x {
                    if grid.get(j, line_start_y + (j - line_start_x)) == '\\' {
                        has_backslash = true;
                        break;
                    }
                }

                if has_backslash {
                    // Mark cells as used
                    for j in line_start_x..=line_end_x {
                        let mark_y = line_start_y + (j - line_start_x);
                        grid.set_used(j, mark_y);
                    }

                    // Apply stretching logic (JS lines 997-1042)
                    let mut adj_start_x = line_start_x as f64;
                    let mut adj_start_y = line_start_y as f64;
                    let mut adj_end_x = line_end_x as f64;
                    let mut adj_end_y = line_end_y as f64;

                    // Upper-left end stretching
                    let top = grid.get(line_start_x, line_start_y);
                    let up = grid.get(line_start_x, line_start_y - 1);
                    let uplt = grid.get(line_start_x - 1, line_start_y - 1);

                    if up == '/'
                        || uplt == '_'
                        || up == '_'
                        || (!is_vertex(top)
                            && (is_solid_h_line(uplt) || is_solid_v_line(uplt)))
                    {
                        // Continue half a cell more to connect
                        adj_start_x -= 0.5;
                        adj_start_y -= 0.5;
                    } else if is_point(uplt) {
                        // Continue 1/4 cell more for points
                        adj_start_x -= 0.25;
                        adj_start_y -= 0.25;
                    } else if top == '\\' && is_solid_d_line_at(grid, line_start_x - 1, line_start_y)
                    {
                        // Cap a sharp vertex: \/ or similar
                        adj_start_x -= 0.5;
                        adj_start_y -= 0.5;
                    }

                    // Lower-right end stretching
                    let bottom = grid.get(line_end_x, line_end_y);
                    let dn = grid.get(line_end_x, line_end_y + 1);
                    let dnrt = grid.get(line_end_x + 1, line_end_y + 1);
                    let rt = grid.get(line_end_x + 1, line_end_y);
                    let lt = grid.get(line_end_x - 1, line_end_y);

                    if dn == '/'
                        || rt == '_'
                        || lt == '_'
                        || (!is_vertex(bottom)
                            && (is_solid_h_line(dnrt) || is_solid_v_line(dnrt)))
                    {
                        // Continue half a cell more to connect
                        adj_end_x += 0.5;
                        adj_end_y += 0.5;
                    } else if is_point(dnrt) {
                        // Continue 1/4 cell more for points
                        adj_end_x += 0.25;
                        adj_end_y += 0.25;
                    } else if bottom == '\\' && is_solid_d_line_at(grid, line_end_x + 1, line_end_y)
                    {
                        // Cap a sharp vertex: /\ or similar
                        adj_end_x += 0.5;
                        adj_end_y += 0.5;
                    }

                    let path = Path::line(
                        Vec2::from_grid_frac(adj_start_x, adj_start_y),
                        Vec2::from_grid_frac(adj_end_x, adj_end_y),
                    );
                    paths.insert(path);
                }
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
    // Scan diagonals from bottom-left to top-right
    // JS: markdeep-diagram.js lines 1058-1131
    // In JS, it scans with x increasing and y decreasing
    let width = grid.width as i32;
    let height = grid.height as i32;

    // JS scans with: for (var x = i, y = grid.height - 1; y >= 0; --y, ++x)
    for i in (-height)..(width) {
        let mut x = i;
        let mut y = height - 1;

        while y >= 0 {
            // Note: JS doesn't check isUsed before starting a diagonal
            // This allows vertices to be shared between backslash and forward-slash diagonals
            if x >= 0 && x < width && is_solid_d_line_at(grid, x, y) {
                // A is bottom-left start, B is top-right end (in JS convention)
                let a_x = x;
                let a_y = y;

                // Find the end
                while x < width && y >= 0 && is_solid_d_line_at(grid, x, y) {
                    x += 1;
                    y -= 1;
                }

                let b_x = x - 1;
                let b_y = y + 1;

                // Check if the line contains at least one actual forward slash
                let mut has_slash = false;
                for j in a_x..=b_x {
                    if grid.get(j, a_y - (j - a_x)) == '/' {
                        has_slash = true;
                        break;
                    }
                }

                if has_slash {
                    // Mark cells as used
                    for j in a_x..=b_x {
                        grid.set_used(j, a_y - (j - a_x));
                    }

                    // Apply stretching logic (JS lines 1073-1127)
                    let mut adj_a_x = a_x as f64; // bottom-left
                    let mut adj_a_y = a_y as f64;
                    let mut adj_b_x = b_x as f64; // top-right
                    let mut adj_b_y = b_y as f64;

                    // Upper-right (B) end stretching
                    let up = grid.get(b_x, b_y - 1);
                    let uprt = grid.get(b_x + 1, b_y - 1);
                    let bottom_b = grid.get(b_x, b_y);

                    if up == '\\'
                        || up == '_'
                        || uprt == '_'
                        || (!is_vertex(grid.get(b_x, b_y))
                            && (is_solid_h_line(uprt) || is_solid_v_line(uprt)))
                    {
                        // Continue half a cell more to connect
                        adj_b_x += 0.5;
                        adj_b_y -= 0.5;
                    } else if is_point(uprt) {
                        // Continue 1/4 cell more for points
                        adj_b_x += 0.25;
                        adj_b_y -= 0.25;
                    }
                    // Note: this is a separate if in JS, not else if
                    if bottom_b == '/' && is_solid_b_line_at(grid, b_x + 1, b_y) {
                        // Cap a sharp vertex: \/ pattern
                        adj_b_x += 0.5;
                        adj_b_y -= 0.5;
                    }

                    // Lower-left (A) end stretching
                    let dn = grid.get(a_x, a_y + 1);
                    let dnlt = grid.get(a_x - 1, a_y + 1);
                    let lt = grid.get(a_x - 1, a_y);
                    let rt = grid.get(a_x + 1, a_y);
                    let top_a = grid.get(a_x, a_y);

                    if dn == '\\'
                        || lt == '_'
                        || rt == '_'
                        || (!is_vertex(grid.get(a_x, a_y))
                            && (is_solid_h_line(dnlt) || is_solid_v_line(dnlt)))
                    {
                        // Continue half a cell more to connect
                        adj_a_x -= 0.5;
                        adj_a_y += 0.5;
                    } else if is_point(dnlt) {
                        // Continue 1/4 cell more for points
                        adj_a_x -= 0.25;
                        adj_a_y += 0.25;
                    } else if top_a == '/' && is_solid_b_line_at(grid, a_x - 1, a_y) {
                        // Cap a sharp vertex: /\ pattern
                        adj_a_x -= 0.5;
                        adj_a_y += 0.5;
                    }

                    // Path goes from A (bottom-left) to B (top-right)
                    let path = Path::line(
                        Vec2::from_grid_frac(adj_a_x, adj_a_y),
                        Vec2::from_grid_frac(adj_b_x, adj_b_y),
                    );
                    paths.insert(path);
                }
            } else {
                x += 1;
                y -= 1;
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
                // JS: Vec2(x + 0.6, y - 1) -> pixel coords ((x + 0.6 + 1) * SCALE, ...)
                let ctrl1 = Vec2::new(
                    (x as f64 + 0.6 + 1.0) * crate::path::SCALE,
                    start.y,
                );
                let ctrl2 = Vec2::new(
                    (x as f64 + 0.6 + 1.0) * crate::path::SCALE,
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
                // JS: Vec2(x - 0.6, y - 1) -> pixel coords ((x - 0.6 + 1) * SCALE, ...)
                let ctrl1 = Vec2::new(
                    (x as f64 - 0.6 + 1.0) * crate::path::SCALE,
                    start.y,
                );
                let ctrl2 = Vec2::new(
                    (x as f64 - 0.6 + 1.0) * crate::path::SCALE,
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

/// Find low horizontal lines marked with underscores
/// JS markdeep-diagram.js lines 1205-1278
fn find_underscore_lines(grid: &mut Grid, paths: &mut PathSet) {
    fn is_ascii_letter(c: char) -> bool {
        c.is_ascii_alphabetic()
    }

    for y in 0..grid.height as i32 {
        let mut x = 0;
        while x < (grid.width as i32) - 2 {
            let lt = grid.get(x - 1, y);
            let c = grid.get(x, y);
            let c1 = grid.get(x + 1, y);
            let c2 = grid.get(x + 2, y);

            // JS: (grid(x, y) === '_') && (grid(x + 1, y) === '_') &&
            //     (!isASCIILetter(grid(x + 2, y)) || (lt === '_')) &&
            //     (!isASCIILetter(lt) || (grid(x + 2, y) === '_'))
            if c == '_'
                && c1 == '_'
                && (!is_ascii_letter(c2) || lt == '_')
                && (!is_ascii_letter(lt) || c2 == '_')
            {
                let ltlt = grid.get(x - 2, y);

                // Start position: Vec2(x - 0.5, y + 0.5)
                let mut a_x = x as f64 - 0.5;

                // Left side extension logic
                if lt == '|'
                    || grid.get(x - 1, y + 1) == '|'
                    || lt == '.'
                    || grid.get(x - 1, y + 1) == '\''
                {
                    // Extend to meet adjacent vertical
                    a_x -= 0.5;

                    // Very special case of overrunning into the side of a curve
                    if lt == '.' && (ltlt == '-' || ltlt == '.') && grid.get(x - 2, y + 1) == '(' {
                        a_x -= 0.5;
                    }
                } else if lt == '/' {
                    a_x -= 1.0;
                }

                // Detect overrun of a tight double curve
                if lt == '(' && ltlt == '(' && grid.get(x, y + 1) == '\'' && grid.get(x, y - 1) == '.'
                {
                    a_x += 0.5;
                }

                // Consume all underscores
                let _start_x = x;
                while x < grid.width as i32 && grid.get(x, y) == '_' {
                    grid.set_used(x, y);
                    x += 1;
                }

                // End position: Vec2(x - 0.5, y + 0.5)
                let mut b_x = x as f64 - 0.5;
                let rt = grid.get(x, y);
                let rt1 = grid.get(x + 1, y);
                let dn = grid.get(x, y + 1);

                // Right side extension logic
                if rt == '|' || dn == '|' || rt == '.' || dn == '\'' {
                    // Extend to meet adjacent vertical
                    b_x += 0.5;

                    // Very special case of overrunning into the side of a curve
                    if rt == '.' && (rt1 == '-' || rt1 == '.') && grid.get(x + 1, y + 1) == ')' {
                        b_x += 0.5;
                    }
                } else if rt == '\\' {
                    b_x += 1.0;
                }

                // Detect overrun of a tight double curve
                if rt == ')'
                    && rt1 == ')'
                    && grid.get(x - 1, y + 1) == '\''
                    && grid.get(x - 1, y - 1) == '.'
                {
                    b_x -= 0.5;
                }

                // Create the path at y + 0.5 (bottom of cell)
                let start = Vec2::from_grid_frac(a_x, y as f64 + 0.5);
                let end = Vec2::from_grid_frac(b_x, y as f64 + 0.5);
                paths.insert(Path::line(start, end));

                // Don't increment x here, we already advanced past the underscores
                continue;
            }
            x += 1;
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
                    // Right arrow - check for horizontal line ending here (rightEndsAt in JS)
                    // or passing through
                    if paths.right_ends_at(x, y) || paths.horizontal_passes_through(x, y) {
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
                    // Left arrow - check for horizontal line ending here (leftEndsAt in JS)
                    // or passing through
                    if paths.left_ends_at(x, y) || paths.horizontal_passes_through(x, y) {
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
                    // Up arrow - JS checks multiple positions due to aspect ratio
                    // First check if line ends at y - 0.5 (between cells)
                    if paths.up_ends_at_frac(x as f64, y as f64 - 0.5) {
                        decorations.insert(Decoration::arrow_frac(x as f64, y as f64 - 0.5, ARROW_UP));
                        grid.set_used(x, y);
                    } else if paths.up_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_UP));
                        grid.set_used(x, y);
                    } else if paths.vertical_passes_through(x, y) {
                        // Line passes through - position at y - 0.5
                        decorations.insert(Decoration::arrow_frac(x as f64, y as f64 - 0.5, ARROW_UP));
                        grid.set_used(x, y);
                    }
                }
                'v' | 'V' => {
                    // Down arrow - JS checks multiple positions due to aspect ratio
                    // First check if line ends at y + 0.5 (between cells)
                    if paths.down_ends_at_frac(x as f64, y as f64 + 0.5) {
                        decorations.insert(Decoration::arrow_frac(x as f64, y as f64 + 0.5, ARROW_DOWN));
                        grid.set_used(x, y);
                    } else if paths.down_ends_at(x, y) {
                        decorations.insert(Decoration::arrow(x, y, ARROW_DOWN));
                        grid.set_used(x, y);
                    } else if paths.vertical_passes_through(x, y) {
                        // Line passes through - position at y + 0.5
                        decorations.insert(Decoration::arrow_frac(x as f64, y as f64 + 0.5, ARROW_DOWN));
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

            // Jump is a ( or ) that bridges a vertical line
            if c == '(' || c == ')' {
                // Check if there's a vertical line above and below
                // Either via paths or direct character check
                let has_line_above = paths.down_ends_at(x, y)
                    || is_solid_v_line(grid.get(x, y - 1))
                    || is_double_v_line(grid.get(x, y - 1));
                let has_line_below = paths.up_ends_at(x, y)
                    || is_solid_v_line(grid.get(x, y + 1))
                    || is_double_v_line(grid.get(x, y + 1));

                if has_line_above && has_line_below {
                    decorations.insert(Decoration::jump(x, y, c));
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
