//! Facet-derived types for SVG parsing and serialization.
//!
//! This crate provides strongly-typed SVG elements that can be deserialized
//! from XML using `facet-xml`.
//!
//! # Example
//!
//! ```rust
//! use facet_svg::Svg;
//!
//! let svg_str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
//!     <rect x="10" y="10" width="80" height="80" fill="blue"/>
//! </svg>"#;
//!
//! let svg: Svg = facet_xml::from_str(svg_str).unwrap();
//! ```

use facet::Facet;
use facet_xml as xml;

/// SVG namespace URI
pub const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Root SVG element
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Svg {
    #[facet(xml::attribute)]
    pub xmlns: Option<String>,
    #[facet(xml::attribute)]
    pub width: Option<String>,
    #[facet(xml::attribute)]
    pub height: Option<String>,
    #[facet(xml::attribute, rename = "viewBox")]
    pub view_box: Option<String>,
    #[facet(xml::elements)]
    pub children: Vec<SvgNode>,
}

/// Any SVG node we care about
#[derive(Facet, Debug, Clone)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
#[repr(u8)]
pub enum SvgNode {
    #[facet(rename = "g")]
    G(Group),
    #[facet(rename = "defs")]
    Defs(Defs),
    #[facet(rename = "style")]
    Style(Style),
    #[facet(rename = "rect")]
    Rect(Rect),
    #[facet(rename = "circle")]
    Circle(Circle),
    #[facet(rename = "ellipse")]
    Ellipse(Ellipse),
    #[facet(rename = "line")]
    Line(Line),
    #[facet(rename = "path")]
    Path(Path),
    #[facet(rename = "polygon")]
    Polygon(Polygon),
    #[facet(rename = "polyline")]
    Polyline(Polyline),
    #[facet(rename = "text")]
    Text(Text),
}

/// SVG group element (`<g>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Group {
    #[facet(xml::attribute)]
    pub id: Option<String>,
    #[facet(xml::attribute)]
    pub class: Option<String>,
    #[facet(xml::attribute)]
    pub transform: Option<String>,
    #[facet(xml::elements)]
    pub children: Vec<SvgNode>,
}

/// SVG defs element (`<defs>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Defs {
    #[facet(xml::elements)]
    pub children: Vec<SvgNode>,
}

/// SVG style element (`<style>`)
#[derive(Facet, Debug, Clone, Default)]
pub struct Style {
    #[facet(xml::attribute, rename = "type")]
    pub type_: Option<String>,
    #[facet(xml::text)]
    pub content: String,
}

/// Common presentation attributes shared by shape elements
pub trait PresentationAttrs {
    fn fill(&self) -> Option<&str>;
    fn stroke(&self) -> Option<&str>;
    fn stroke_width(&self) -> Option<&str>;
    fn stroke_dasharray(&self) -> Option<&str>;
    fn style(&self) -> Option<&str>;
}

macro_rules! impl_presentation_attrs {
    ($($ty:ty),*) => {
        $(
            impl PresentationAttrs for $ty {
                fn fill(&self) -> Option<&str> { self.fill.as_deref() }
                fn stroke(&self) -> Option<&str> { self.stroke.as_deref() }
                fn stroke_width(&self) -> Option<&str> { self.stroke_width.as_deref() }
                fn stroke_dasharray(&self) -> Option<&str> { self.stroke_dasharray.as_deref() }
                fn style(&self) -> Option<&str> { self.style.as_deref() }
            }
        )*
    };
}

/// SVG rect element (`<rect>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Rect {
    #[facet(xml::attribute)]
    pub x: Option<f64>,
    #[facet(xml::attribute)]
    pub y: Option<f64>,
    #[facet(xml::attribute)]
    pub width: Option<f64>,
    #[facet(xml::attribute)]
    pub height: Option<f64>,
    #[facet(xml::attribute)]
    pub rx: Option<f64>,
    #[facet(xml::attribute)]
    pub ry: Option<f64>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG circle element (`<circle>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Circle {
    #[facet(xml::attribute)]
    pub cx: Option<f64>,
    #[facet(xml::attribute)]
    pub cy: Option<f64>,
    #[facet(xml::attribute)]
    pub r: Option<f64>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG ellipse element (`<ellipse>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Ellipse {
    #[facet(xml::attribute)]
    pub cx: Option<f64>,
    #[facet(xml::attribute)]
    pub cy: Option<f64>,
    #[facet(xml::attribute)]
    pub rx: Option<f64>,
    #[facet(xml::attribute)]
    pub ry: Option<f64>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG line element (`<line>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Line {
    #[facet(xml::attribute)]
    pub x1: Option<f64>,
    #[facet(xml::attribute)]
    pub y1: Option<f64>,
    #[facet(xml::attribute)]
    pub x2: Option<f64>,
    #[facet(xml::attribute)]
    pub y2: Option<f64>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG path element (`<path>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Path {
    #[facet(xml::attribute)]
    pub d: Option<String>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG polygon element (`<polygon>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Polygon {
    #[facet(xml::attribute)]
    pub points: Option<String>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG polyline element (`<polyline>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Polyline {
    #[facet(xml::attribute)]
    pub points: Option<String>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute, rename = "stroke-dasharray")]
    pub stroke_dasharray: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
}

/// SVG text element (`<text>`)
#[derive(Facet, Debug, Clone, Default)]
#[facet(xml::ns_all = "http://www.w3.org/2000/svg")]
pub struct Text {
    #[facet(xml::attribute)]
    pub x: Option<f64>,
    #[facet(xml::attribute)]
    pub y: Option<f64>,
    #[facet(xml::attribute)]
    pub fill: Option<String>,
    #[facet(xml::attribute)]
    pub stroke: Option<String>,
    #[facet(xml::attribute, rename = "stroke-width")]
    pub stroke_width: Option<String>,
    #[facet(xml::attribute)]
    pub style: Option<String>,
    #[facet(xml::attribute, rename = "text-anchor")]
    pub text_anchor: Option<String>,
    #[facet(xml::attribute, rename = "dominant-baseline")]
    pub dominant_baseline: Option<String>,
    #[facet(xml::text)]
    pub content: String,
}

impl_presentation_attrs!(Rect, Circle, Ellipse, Line, Path, Polygon, Polyline);

impl PresentationAttrs for Text {
    fn fill(&self) -> Option<&str> {
        self.fill.as_deref()
    }
    fn stroke(&self) -> Option<&str> {
        self.stroke.as_deref()
    }
    fn stroke_width(&self) -> Option<&str> {
        self.stroke_width.as_deref()
    }
    fn stroke_dasharray(&self) -> Option<&str> {
        None // Text doesn't have stroke-dasharray
    }
    fn style(&self) -> Option<&str> {
        self.style.as_deref()
    }
}

// Re-export facet_xml for convenience
pub use facet_xml;
