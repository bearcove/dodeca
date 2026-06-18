//! Rendering context - tracks state during rendering

use std::collections::HashMap;

use crate::ast::Direction;
use crate::types::{EvalValue, Length as Inches};

use super::expand_object_bounds;
use super::types::*;

/// Rendering context
pub struct RenderContext {
    /// Current direction
    pub direction: Direction,
    /// Current position (where the next object will be placed)
    pub position: PointIn,
    /// Objects with explicit names (e.g., `C1: circle`)
    /// cref: pik_find_byname (pikchr.c:4027-4032) - first pass looks for zName match
    pub explicit_names: HashMap<String, RenderedObject>,
    /// Objects with names derived from text content (e.g., `circle "C0"`)
    /// cref: pik_find_byname (pikchr.c:4034-4044) - second pass looks for text match
    pub text_names: HashMap<String, RenderedObject>,
    /// All objects in order
    pub object_list: Vec<RenderedObject>,
    /// Variables (typed: lengths, scalars, colors)
    pub variables: HashMap<String, EvalValue>,
    /// Bounding box of all objects
    pub bounds: BoundingBox,
    /// Current object being constructed (for `this` keyword support)
    pub current_object: Option<RenderedObject>,
    /// Macro definitions (name -> body)
    pub macros: HashMap<String, String>,
    /// Named positions (e.g., `OUT: 6.3in right of previous.e`)
    /// cref: Labeled positions are stored separately from objects
    pub named_positions: HashMap<String, PointIn>,
}

impl Default for RenderContext {
    fn default() -> Self {
        let mut ctx = Self {
            direction: Direction::Right,
            position: pin(0.0, 0.0),
            explicit_names: HashMap::new(),
            text_names: HashMap::new(),
            object_list: Vec::new(),
            variables: HashMap::new(),
            bounds: BoundingBox::new(),
            current_object: None,
            macros: HashMap::new(),
            named_positions: HashMap::new(),
        };
        ctx.init_builtin_variables();
        ctx
    }
}

impl RenderContext {
    pub fn new() -> Self {
        Self::default()
    }

    fn init_builtin_variables(&mut self) {
        // Built-in variables mirror pikchr.c aBuiltin[]
        // These are the default values that should be available in all pikchr scripts
        macro_rules! builtin_vars {
            ($($name:ident => $value:expr),* $(,)?) => {
                $(
                    self.variables.insert(stringify!($name).to_string(), $value);
                )*
            };
        }

        // Built-in variables mirror pikchr.c aBuiltin[]
        // Names must match C exactly for compatibility
        builtin_vars! {
            // Arc
            arcrad     => EvalValue::Length(Inches::from(0.25)),
            // Arrow
            arrowht    => EvalValue::Length(Inches::from(0.08)),  // C name
            arrowwid   => EvalValue::Length(Inches::from(0.06)),
            // Box
            boxht      => EvalValue::Length(Inches::from(0.5)),   // C name
            boxwid     => EvalValue::Length(Inches::from(0.75)),  // C name
            boxrad     => EvalValue::Length(Inches::from(0.0)),
            // Character/text metrics
            charht     => EvalValue::Scalar(0.14),
            charwid    => EvalValue::Scalar(0.08),
            // Circle
            circlerad  => EvalValue::Length(Inches::from(0.25)),
            // Cylinder
            cylht      => EvalValue::Length(Inches::from(0.5)),
            cylwid     => EvalValue::Length(Inches::from(0.75)),
            cylrad     => EvalValue::Length(Inches::from(0.075)),
            // Dash
            dashwid    => EvalValue::Length(Inches::from(0.05)),
            // Diamond
            diamondht  => EvalValue::Length(Inches::from(0.75)), // C name
            diamondwid => EvalValue::Length(Inches::from(1.0)),  // C name
            // Dot
            dotrad     => EvalValue::Length(Inches::from(0.015)),  // cref: pikchr.c:3669
            // Ellipse
            ellipseht  => EvalValue::Length(Inches::from(0.5)),
            ellipsewid => EvalValue::Length(Inches::from(0.75)),
            // File
            fileht     => EvalValue::Length(Inches::from(0.75)), // C name
            filewid    => EvalValue::Length(Inches::from(0.5)),  // C name
            filerad    => EvalValue::Length(Inches::from(0.15)),
            // Line
            lineht     => EvalValue::Length(Inches::from(0.5)),
            linewid    => EvalValue::Length(Inches::from(0.5)),
            linerad    => EvalValue::Length(Inches::from(0.0)),
            // Move
            movewid    => EvalValue::Length(Inches::from(0.5)),
            // Oval
            ovalht     => EvalValue::Length(Inches::from(0.5)),  // C name
            ovalwid    => EvalValue::Length(Inches::from(1.0)),  // C name
            // Scale
            scale      => EvalValue::Scalar(1.0),
            fontscale  => EvalValue::Scalar(1.0),  // cref: pikchr.c aBuiltin[] - global font scale multiplier
            // Text
            textht     => EvalValue::Length(Inches::from(0.5)),
            textwid    => EvalValue::Length(Inches::from(0.75)),
            // Thickness/stroke
            thickness  => EvalValue::Length(Inches::from(0.015)),
        }
    }

    /// Get the last rendered object
    pub fn last_object(&self) -> Option<&RenderedObject> {
        self.object_list.last()
    }

    /// Get an object by name
    ///
    /// cref: pik_find_byname (pikchr.c:4014-4047)
    /// First looks for explicitly named objects (e.g., `C1: circle`)
    /// Then falls back to objects with matching text content (e.g., `circle "C0"`)
    pub fn get_object(&self, name: &str) -> Option<&RenderedObject> {
        // First pass: look for explicitly tagged objects
        if let Some(obj) = self.explicit_names.get(name) {
            return Some(obj);
        }
        // Second pass: look for objects with matching text content
        self.text_names.get(name)
    }

    /// Get the nth object of a class (1-indexed, from start)
    pub fn get_nth_object(&self, n: usize, class: Option<ClassName>) -> Option<&RenderedObject> {
        let filtered: Vec<_> = self
            .object_list
            .iter()
            .filter(|o| class.map(|c| o.class() == c).unwrap_or(true))
            .collect();
        filtered.get(n.saturating_sub(1)).copied()
    }

    /// Get the nth last object of a class (1-indexed, from end)
    /// e.g., "3rd last box" gets the 3rd box counting from the end
    pub fn get_nth_last_object(
        &self,
        n: usize,
        class: Option<ClassName>,
    ) -> Option<&RenderedObject> {
        let filtered: Vec<_> = self
            .object_list
            .iter()
            .filter(|o| class.map(|c| o.class() == c).unwrap_or(true))
            .collect();
        let len = filtered.len();
        if n > len || n == 0 {
            return None;
        }
        filtered.get(len - n).copied()
    }

    /// Get the last object of a class
    pub fn get_last_object(&self, class: Option<ClassName>) -> Option<&RenderedObject> {
        self.object_list
            .iter()
            .rev()
            .find(|o| class.map(|c| o.class() == c).unwrap_or(true))
    }

    /// Get a scalar value from variables, with fallback
    // cref: pik_value (pikchr.c:6102)
    pub fn get_scalar(&self, name: &str, default: f64) -> f64 {
        self.variables
            .get(name)
            .map(|v| v.as_scalar())
            .unwrap_or(default)
    }

    /// Get a length value from variables, with fallback
    /// cref: pik_value (pikchr.c:6102)
    /// Accepts both Length and Scalar values - scalars are interpreted as inches
    pub fn get_length(&self, name: &str, default: f64) -> Inches {
        self.variables
            .get(name)
            .map(|v| match v {
                EvalValue::Length(l) => *l,
                EvalValue::Scalar(s) => Inches(*s),
                EvalValue::Color(_) => Inches(default),
            })
            .unwrap_or(Inches(default))
    }

    /// Move position in the current direction
    pub fn advance(&mut self, distance: Inches) {
        self.position += self.direction.offset(distance);
    }

    /// Add an object to the context
    // cref: pik_after_adding_element (pikchr.c:7095) - p->eDir = pObj->outDir
    #[allow(unused_variables)]
    pub fn add_object(&mut self, obj: RenderedObject) {
        let old_cursor = self.position;

        // Update bounds
        expand_object_bounds(&mut self.bounds, &obj);

        // Update direction to match the object's direction
        // This handles cases like "arrow left" where the direction attribute
        // changes the global direction for subsequent objects
        self.direction = obj.direction;

        // Update position to exit edge of object in the object's direction
        // For shaped objects, this is the edge point in the travel direction
        // For line-like objects, this is already handled correctly by their end()
        let exit_point = match obj.class() {
            ClassName::Line
            | ClassName::Arrow
            | ClassName::Spline
            | ClassName::Move
            | ClassName::Arc => {
                // Check if this is a closed line (polygon)
                // cref: pikchr.c:7122-7126 - closed lines use bbox edge as exit
                let is_closed = obj.style().close_path;
                if is_closed
                    && matches!(
                        obj.class(),
                        ClassName::Line | ClassName::Arrow | ClassName::Spline
                    )
                {
                    // For closed lines, exit point is edge of bounding box in object's direction
                    // cref: pik_elem_set_exit (pikchr.c:5740-5747)
                    use crate::types::UnitVec;
                    let unit_dir = match self.direction {
                        crate::ast::Direction::Right => UnitVec::EAST,
                        crate::ast::Direction::Left => UnitVec::WEST,
                        crate::ast::Direction::Up => UnitVec::NORTH,
                        crate::ast::Direction::Down => UnitVec::SOUTH,
                    };
                    obj.edge_point(unit_dir)
                } else {
                    // For open lines, exit point is last waypoint
                    obj.end()
                }
            }
            ClassName::Dot => {
                // cref: dotCheck (pikchr.c:4042-4047)
                // Dots set w = h = 0, so ptExit = ptAt (center, not edge)
                // This means dots don't advance the cursor by their radius
                obj.center()
            }
            _ => {
                // For shaped objects, get edge point in current direction
                use crate::types::UnitVec;
                let unit_dir = match self.direction {
                    crate::ast::Direction::Right => UnitVec::EAST,
                    crate::ast::Direction::Left => UnitVec::WEST,
                    crate::ast::Direction::Up => UnitVec::NORTH,
                    crate::ast::Direction::Down => UnitVec::SOUTH,
                };
                obj.edge_point(unit_dir)
            }
        };
        self.position = exit_point;

        crate::log::debug!(
            "{:?} {:?}: cursor ({:.3}, {:.3}) -> ({:.3}, {:.3})",
            obj.class(),
            obj.name.as_deref().unwrap_or("-"),
            old_cursor.x.0,
            old_cursor.y.0,
            exit_point.x.0,
            exit_point.y.0,
        );

        // Store named objects in the appropriate lookup map
        // cref: pik_find_byname (pikchr.c:4027-4044)
        // Explicit names (from labels like `C1:`) go in explicit_names
        // Text-derived names (from `circle "C0"`) go in text_names
        // An object can have BOTH - e.g., `B1: box "One"` is findable by "B1" and "One"
        if let Some(ref name) = obj.name {
            if obj.name_is_explicit {
                self.explicit_names.insert(name.clone(), obj.clone());
            } else {
                self.text_names.insert(name.clone(), obj.clone());
            }
        }
        // Also store text-derived name separately if it exists (for objects like `B1: box "One"`)
        if let Some(ref text_name) = obj.text_name {
            self.text_names.insert(text_name.clone(), obj.clone());
        }

        self.object_list.push(obj);
    }

    /// Add a named position (e.g., `OUT: 6.3in right of previous.e`)
    pub fn add_named_position(&mut self, name: String, pos: PointIn) {
        crate::log::debug!(
            name = %name,
            x = pos.x.raw(),
            y = pos.y.raw(),
            "Adding named position"
        );
        self.named_positions.insert(name, pos);
    }

    /// Get a named position
    pub fn get_named_position(&self, name: &str) -> Option<PointIn> {
        self.named_positions.get(name).copied()
    }
}
