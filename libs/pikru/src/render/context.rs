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
        // TODO: Move the big init_builtin_variables implementation here
        // Built-in length defaults mirror pikchr.c aBuiltin[]
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
            .filter(|o| class.map(|c| o.class == c).unwrap_or(true))
            .collect();
        filtered.get(n.saturating_sub(1)).copied()
    }

    /// Get the last object of a class
    pub fn get_last_object(&self, class: Option<ObjectClass>) -> Option<&RenderedObject> {
        self.object_list
            .iter()
            .rev()
            .find(|o| class.map(|c| o.class == c).unwrap_or(true))
    }

    /// Move position in the current direction
    pub fn advance(&mut self, distance: Inches) {
        self.position = self.position + self.direction.offset(distance);
    }

    /// Add an object to the context
    pub fn add_object(&mut self, obj: RenderedObject) {
        // Update bounds
        expand_object_bounds(&mut self.bounds, &obj);

        // Update position to the exit point of the object
        self.position = obj.end;

        // Store named objects
        if let Some(ref name) = obj.name {
            self.objects.insert(name.clone(), obj.clone());
        }

        self.object_list.push(obj);
    }
}
