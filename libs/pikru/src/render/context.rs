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
    /// Named objects for reference
    pub objects: HashMap<String, RenderedObject>,
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
}

impl Default for RenderContext {
    fn default() -> Self {
        let mut ctx = Self {
            direction: Direction::Right,
            position: pin(0.0, 0.0),
            objects: HashMap::new(),
            object_list: Vec::new(),
            variables: HashMap::new(),
            bounds: BoundingBox::new(),
            current_object: None,
            macros: HashMap::new(),
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
            cylrad     => EvalValue::Length(Inches::from(0.1)),
            // Dash
            dashwid    => EvalValue::Length(Inches::from(0.05)),
            // Diamond
            diamondht  => EvalValue::Length(Inches::from(0.75)), // C name
            diamondwid => EvalValue::Length(Inches::from(1.0)),  // C name
            // Dot
            dotrad     => EvalValue::Length(Inches::from(0.025)),
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
    pub fn get_object(&self, name: &str) -> Option<&RenderedObject> {
        self.objects.get(name)
    }

    /// Get the nth object of a class (1-indexed)
    pub fn get_nth_object(&self, n: usize, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        let filtered: Vec<_> = self
            .object_list
            .iter()
            .filter(|o| class.map(|c| o.class() == c).unwrap_or(true))
            .collect();
        filtered.get(n.saturating_sub(1)).copied()
    }

    /// Get the last object of a class
    pub fn get_last_object(&self, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        self.object_list
            .iter()
            .rev()
            .find(|o| class.map(|c| o.class() == c).unwrap_or(true))
    }

    /// Get a scalar value from variables, with fallback
    pub fn get_scalar(&self, name: &str, default: f64) -> f64 {
        self.variables
            .get(name)
            .map(|v| v.as_scalar())
            .unwrap_or(default)
    }

    /// Get a length value from variables, with fallback
    pub fn get_length(&self, name: &str, default: f64) -> Inches {
        self.variables
            .get(name)
            .and_then(|v| v.as_length())
            .unwrap_or(Inches(default))
    }

    /// Move position in the current direction
    pub fn advance(&mut self, distance: Inches) {
        self.position = self.position + self.direction.offset(distance);
    }

    /// Add an object to the context
    pub fn add_object(&mut self, obj: RenderedObject) {
        // Update bounds
        expand_object_bounds(&mut self.bounds, &obj);

        // Update position to exit edge of object in current direction
        // For shaped objects, this is the edge point in the travel direction
        // For line-like objects, this is already handled correctly by their end()
        let exit_point = match obj.class() {
            ObjectClass::Line | ObjectClass::Arrow | ObjectClass::Spline | ObjectClass::Move => {
                // For line-like objects, end() is correct
                obj.end()
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

        // Store named objects
        if let Some(ref name) = obj.name {
            self.objects.insert(name.clone(), obj.clone());
        }

        self.object_list.push(obj);
    }
}
