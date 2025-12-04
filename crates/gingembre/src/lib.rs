// Template engine is a work in progress - many AST fields are for future error reporting
#![allow(dead_code)]

//! gingembre - A Jinja-like template engine with rich diagnostics
//!
//! A template language featuring:
//! - Rich diagnostics via miette
//! - Parse once, run many times (compiled templates)
//! - Template inheritance and includes
//! - Macro system with imports
//!
//! # Syntax Overview
//!
//! ```text
//! {{ expr }}              - Expression interpolation
//! {% if cond %}...{% endif %}     - Conditionals
//! {% for item in items %}...{% endfor %}  - Loops
//! {{ value | filter }}    - Filters
//! {{ object.field }}      - Field access
//! {% extends "base.html" %}       - Template inheritance
//! {% block name %}...{% endblock %} - Block definitions
//! {% include "partial.html" %}    - Template includes
//! {% macro name(args) %}...{% endmacro %} - Macro definitions
//! ```
//!
//! # Example
//!
//! ```ignore
//! use gingembre::{Engine, Context, Value, InMemoryLoader};
//!
//! let mut loader = InMemoryLoader::new();
//! loader.add("hello.html", "Hello, {{ name }}!");
//!
//! let engine = Engine::new(loader);
//! let mut ctx = Context::new();
//! ctx.set("name", Value::String("World".into()));
//!
//! let output = engine.render("hello.html", &ctx)?;
//! assert_eq!(output, "Hello, World!");
//! ```

pub mod ast;
mod error;
mod eval;
mod lazy;
pub mod lexer;
pub mod parser;
mod render;

pub use eval::{Context, GlobalFn, Value, ValueExt};
pub use lazy::{DataPath, DataResolver, LazyValue};
pub use render::{Engine, InMemoryLoader, TemplateLoader};

// Re-export facet_value types for convenience
pub use facet_value::{VArray, VObject, VSafeString, VString};
