//! Typed HTML element definitions for use with `hotmeal`.
//!
//! This crate provides Facet-derived types for all standard HTML5 elements,
//! allowing you to parse HTML documents with full type safety.
//!
//! # Quick Start
//!
//! ```rust
//! use hotmeal::{Html, Body, Div, P, FlowContent};
//!
//! // Parse an HTML document
//! let html_source = r#"
//!     <!DOCTYPE html>
//!     <html>
//!         <body>
//!             <div class="container">
//!                 <p>Hello, world!</p>
//!             </div>
//!         </body>
//!     </html>
//! "#;
//!
//! let doc: Html = hotmeal::parse(html_source);
//!
//! // Access the parsed structure
//! if let Some(body) = &doc.body {
//!     for child in &body.children {
//!         if let FlowContent::Div(div) = child {
//!             println!("Found div with class: {:?}", div.attrs.class);
//!         }
//!     }
//! }
//! ```
//!
//! # Content Models
//!
//! HTML elements are organized by their content model:
//!
//! - [`FlowContent`] - Block and inline elements that can appear in `<body>`, `<div>`, etc.
//! - [`PhrasingContent`] - Inline elements that can appear in `<p>`, `<span>`, `<a>`, etc.
//!
//! These enums allow mixed content with proper nesting validation at the type level.
//!
//! # Global Attributes
//!
//! All elements include [`GlobalAttrs`] via the `attrs` field, which provides:
//! - Standard attributes: `id`, `class`, `style`, `lang`, `dir`, etc.
//! - Common event handlers: `onclick`, `onchange`, `onfocus`, etc.
//! - An `extra` field that captures unknown attributes like `data-*` and `aria-*`
//!
//! # Custom Elements
//!
//! Unknown HTML elements (like `<my-component>` or syntax highlighting tags like `<a-k>`)
//! are captured via [`CustomElement`] and [`CustomPhrasingElement`], preserving their
//! tag names, attributes, and children during parse/serialize roundtrips.
//!
//! # Element Categories
//!
//! Elements are organized by category:
//! - **Document**: [`Html`], [`Head`], [`Body`]
//! - **Metadata**: [`Title`], [`Base`], [`Link`], [`Meta`], [`Style`]
//! - **Sections**: [`Header`], [`Footer`], [`Main`], [`Article`], [`Section`], [`Nav`], [`Aside`]
//! - **Headings**: [`H1`], [`H2`], [`H3`], [`H4`], [`H5`], [`H6`]
//! - **Grouping**: [`P`], [`Div`], [`Span`], [`Pre`], [`Blockquote`], [`Ol`], [`Ul`], [`Li`], etc.
//! - **Text-level**: [`A`], [`Em`], [`Strong`], [`Code`], [`Br`], [`Wbr`], etc.
//! - **Embedded**: [`Img`], [`Iframe`], [`Video`], [`Audio`], [`Source`], [`Picture`]
//! - **Tables**: [`Table`], [`Thead`], [`Tbody`], [`Tr`], [`Th`], [`Td`], etc.
//! - **Forms**: [`Form`], [`Input`], [`Button`], [`Select`], [`OptionElement`], [`Textarea`], [`Label`]
//! - **Interactive**: [`Details`], [`Summary`], [`Dialog`]
//! - **Scripting**: [`Script`], [`Noscript`], [`Template`], [`Canvas`]

use facet::Facet;

// =============================================================================
// Global Attributes (common to all HTML elements)
// =============================================================================

/// Global attributes that can appear on any HTML element.
///
/// This includes standard HTML global attributes and common event handlers.
/// Unknown attributes (like data-*, aria-*, and less common event handlers)
/// are captured in the `extra` field.
#[derive(Debug, Default, Facet)]
#[facet(default, skip_all_unless_truthy)]
pub struct GlobalAttrs {
    // Standard global attributes
    /// Unique identifier for the element.
    #[facet(attribute, default)]
    pub id: Option<String>,
    /// CSS class names.
    #[facet(attribute, default)]
    pub class: Option<String>,
    /// Inline CSS styles.
    #[facet(attribute, default)]
    pub style: Option<String>,
    /// Advisory title/tooltip.
    /// Note: Named `tooltip` in Rust to avoid collision with `<title>` child element in Head.
    /// Serializes as the `title` HTML attribute.
    #[facet(attribute, default, rename = "title")]
    pub tooltip: Option<String>,
    /// Language of the element's content.
    #[facet(attribute, default)]
    pub lang: Option<String>,
    /// Text directionality (ltr, rtl, auto).
    #[facet(attribute, default)]
    pub dir: Option<String>,
    /// Whether the element is hidden.
    #[facet(attribute, default)]
    pub hidden: Option<String>,
    /// Tab order of the element.
    #[facet(attribute, default)]
    pub tabindex: Option<String>,
    /// Access key for the element.
    #[facet(attribute, default)]
    pub accesskey: Option<String>,
    /// Whether the element is draggable.
    #[facet(attribute, default)]
    pub draggable: Option<String>,
    /// Whether the element is editable.
    #[facet(attribute, default)]
    pub contenteditable: Option<String>,
    /// Whether spellchecking is enabled.
    #[facet(attribute, default)]
    pub spellcheck: Option<String>,
    /// Whether the element should be translated.
    #[facet(attribute, default)]
    pub translate: Option<String>,
    /// ARIA role.
    #[facet(attribute, default)]
    pub role: Option<String>,

    // Common event handlers (most frequently used)
    /// Script to run on mouse click.
    #[facet(attribute, default)]
    pub onclick: Option<String>,
    /// Script to run on mouse double-click.
    #[facet(attribute, default)]
    pub ondblclick: Option<String>,
    /// Script to run when mouse button is pressed.
    #[facet(attribute, default)]
    pub onmousedown: Option<String>,
    /// Script to run when mouse pointer moves over element.
    #[facet(attribute, default)]
    pub onmouseover: Option<String>,
    /// Script to run when mouse pointer moves out of element.
    #[facet(attribute, default)]
    pub onmouseout: Option<String>,
    /// Script to run when mouse button is released.
    #[facet(attribute, default)]
    pub onmouseup: Option<String>,
    /// Script to run when mouse enters element.
    #[facet(attribute, default)]
    pub onmouseenter: Option<String>,
    /// Script to run when mouse leaves element.
    #[facet(attribute, default)]
    pub onmouseleave: Option<String>,
    /// Script to run when key is pressed down.
    #[facet(attribute, default)]
    pub onkeydown: Option<String>,
    /// Script to run when key is released.
    #[facet(attribute, default)]
    pub onkeyup: Option<String>,
    /// Script to run when element receives focus.
    #[facet(attribute, default)]
    pub onfocus: Option<String>,
    /// Script to run when element loses focus.
    #[facet(attribute, default)]
    pub onblur: Option<String>,
    /// Script to run when value changes.
    #[facet(attribute, default)]
    pub onchange: Option<String>,
    /// Script to run on input.
    #[facet(attribute, default)]
    pub oninput: Option<String>,
    /// Script to run when form is submitted.
    #[facet(attribute, default)]
    pub onsubmit: Option<String>,
    /// Script to run when resource is loaded.
    #[facet(attribute, default)]
    pub onload: Option<String>,
    /// Script to run when error occurs.
    #[facet(attribute, default)]
    pub onerror: Option<String>,
    /// Script to run when element is scrolled.
    #[facet(attribute, default)]
    pub onscroll: Option<String>,
    /// Script to run on context menu (right-click).
    #[facet(attribute, default)]
    pub oncontextmenu: Option<String>,

    // Catch-all for unknown attributes (data-*, aria-*, less common events, etc.)
    /// Extra attributes not explicitly modeled.
    /// Includes data-* attributes, aria-* attributes, and less common event handlers.
    /// Keys are the full attribute names as they appear in HTML.
    /// Uses BTreeMap for deterministic serialization order.
    #[facet(flatten, default)]
    pub extra: std::collections::BTreeMap<String, String>,
}

// =============================================================================
// Document Structure
// =============================================================================

/// The root HTML document element.
#[derive(Debug, Default, Facet)]
#[facet(rename = "html")]
pub struct Html {
    /// DOCTYPE declaration name (e.g., "html" for `<!DOCTYPE html>`).
    /// When present, the serializer will emit `<!DOCTYPE {name}>` before the html element.
    /// Set to `Some("html".to_string())` for standard HTML5 documents.
    /// This is handled specially by the HTML parser/serializer using the "doctype" pseudo-attribute.
    #[facet(attribute, default)]
    pub doctype: Option<String>,
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Document head.
    #[facet(default)]
    pub head: Option<Head>,
    /// Document body.
    ///
    /// When marked with `other`, this field acts as a fallback: if the root element
    /// is not `<html>`, the content is deserialized into this Body field instead.
    /// This enables parsing HTML fragments (like `<div>...</div>`) into the `Html` type.
    #[facet(other, default)]
    pub body: Option<Body>,
}

/// The document head containing metadata.
#[derive(Debug, Default, Facet)]
#[facet(rename = "head")]
pub struct Head {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (metadata content).
    #[facet(flatten, default)]
    pub children: Vec<MetadataContent>,
}

impl Head {
    /// Get the title element if present.
    pub fn title(&self) -> Option<&Title> {
        self.children.iter().find_map(|c| match c {
            MetadataContent::Title(t) => Some(t),
            _ => None,
        })
    }

    /// Get all meta elements.
    pub fn meta(&self) -> impl Iterator<Item = &Meta> {
        self.children.iter().filter_map(|c| match c {
            MetadataContent::Meta(m) => Some(m),
            _ => None,
        })
    }

    /// Get all link elements.
    pub fn links(&self) -> impl Iterator<Item = &Link> {
        self.children.iter().filter_map(|c| match c {
            MetadataContent::Link(l) => Some(l),
            _ => None,
        })
    }

    /// Get all style elements.
    pub fn styles(&self) -> impl Iterator<Item = &Style> {
        self.children.iter().filter_map(|c| match c {
            MetadataContent::Style(s) => Some(s),
            _ => None,
        })
    }

    /// Get all script elements.
    pub fn scripts(&self) -> impl Iterator<Item = &Script> {
        self.children.iter().filter_map(|c| match c {
            MetadataContent::Script(s) => Some(s),
            _ => None,
        })
    }
}

/// The document body containing visible content.
#[derive(Debug, Default, Facet)]
#[facet(rename = "body")]
pub struct Body {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (mixed content).
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

// =============================================================================
// Metadata Elements
// =============================================================================

/// The document title.
#[derive(Debug, Default, Facet)]
#[facet(rename = "title")]
pub struct Title {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Text content of the title.
    #[facet(text, default)]
    pub text: String,
}

/// Base URL for relative URLs.
#[derive(Debug, Default, Facet)]
#[facet(rename = "base")]
pub struct Base {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Base URL.
    #[facet(attribute, default)]
    pub href: Option<String>,
    /// Default browsing context.
    #[facet(attribute, default)]
    pub target: Option<String>,
}

/// External resource link.
#[derive(Debug, Default, Facet)]
#[facet(rename = "link")]
pub struct Link {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// URL of the linked resource.
    #[facet(attribute, default)]
    pub href: Option<String>,
    /// Relationship type.
    #[facet(attribute, default)]
    pub rel: Option<String>,
    /// MIME type of the resource.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Media query for the resource.
    #[facet(attribute, default)]
    pub media: Option<String>,
    /// Integrity hash.
    #[facet(attribute, default)]
    pub integrity: Option<String>,
    /// Crossorigin attribute.
    #[facet(attribute, default)]
    pub crossorigin: Option<String>,
    /// Resource sizes (for icons).
    #[facet(attribute, default)]
    pub sizes: Option<String>,
    /// Alternative stylesheet title.
    #[facet(attribute, default, rename = "as")]
    pub as_: Option<String>,
}

/// Document metadata.
#[derive(Debug, Default, Facet)]
#[facet(rename = "meta")]
pub struct Meta {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Metadata name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Metadata content.
    #[facet(attribute, default)]
    pub content: Option<String>,
    /// Character encoding.
    #[facet(attribute, default)]
    pub charset: Option<String>,
    /// Pragma directive.
    #[facet(attribute, default, rename = "http-equiv")]
    pub http_equiv: Option<String>,
    /// Property (for Open Graph, etc.).
    #[facet(attribute, default)]
    pub property: Option<String>,
}

/// Inline stylesheet.
#[derive(Debug, Default, Facet)]
#[facet(rename = "style")]
pub struct Style {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Media query.
    #[facet(attribute, default)]
    pub media: Option<String>,
    /// MIME type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// CSS content.
    #[facet(text, default)]
    pub text: String,
}

// =============================================================================
// Section Elements
// =============================================================================

/// Page header.
#[derive(Debug, Default, Facet)]
#[facet(rename = "header")]
pub struct Header {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Page or section footer.
#[derive(Debug, Default, Facet)]
#[facet(rename = "footer")]
pub struct Footer {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Main content area.
#[derive(Debug, Default, Facet)]
#[facet(rename = "main")]
pub struct Main {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Self-contained article.
#[derive(Debug, Default, Facet)]
#[facet(rename = "article")]
pub struct Article {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Generic section.
#[derive(Debug, Default, Facet)]
#[facet(rename = "section")]
pub struct Section {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Navigation section.
#[derive(Debug, Default, Facet)]
#[facet(rename = "nav")]
pub struct Nav {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Sidebar content.
#[derive(Debug, Default, Facet)]
#[facet(rename = "aside")]
pub struct Aside {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Address/contact information.
#[derive(Debug, Default, Facet)]
#[facet(rename = "address")]
pub struct Address {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

// =============================================================================
// Heading Elements
// =============================================================================

/// Level 1 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h1")]
pub struct H1 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Level 2 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h2")]
pub struct H2 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Level 3 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h3")]
pub struct H3 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Level 4 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h4")]
pub struct H4 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Level 5 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h5")]
pub struct H5 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Level 6 heading.
#[derive(Debug, Default, Facet)]
#[facet(rename = "h6")]
pub struct H6 {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

// =============================================================================
// Grouping Content
// =============================================================================

/// Paragraph.
#[derive(Debug, Default, Facet)]
#[facet(rename = "p")]
pub struct P {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Generic container (block).
#[derive(Debug, Default, Facet)]
#[facet(rename = "div")]
pub struct Div {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Generic container (inline).
#[derive(Debug, Default, Facet)]
#[facet(rename = "span")]
pub struct Span {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Preformatted text.
#[derive(Debug, Default, Facet)]
#[facet(rename = "pre")]
pub struct Pre {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Block quotation.
#[derive(Debug, Default, Facet)]
#[facet(rename = "blockquote")]
pub struct Blockquote {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Citation URL.
    #[facet(attribute, default)]
    pub cite: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Ordered list.
#[derive(Debug, Default, Facet)]
#[facet(rename = "ol")]
pub struct Ol {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Starting number.
    #[facet(attribute, default)]
    pub start: Option<String>,
    /// Numbering type (1, a, A, i, I).
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Reversed order.
    #[facet(attribute, default)]
    pub reversed: Option<String>,
    /// Child elements (li elements and whitespace text nodes).
    #[facet(flatten, default)]
    pub children: Vec<OlContent>,
}

/// Content types that can appear inside an ordered list.
///
/// Includes text nodes to preserve whitespace between list items.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum OlContent {
    /// Text node (for whitespace between list items).
    #[facet(text)]
    Text(String),
    /// List item.
    #[facet(rename = "li")]
    Li(Li),
}

/// Unordered list.
#[derive(Debug, Default, Facet)]
#[facet(rename = "ul")]
pub struct Ul {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (li elements and whitespace text nodes).
    #[facet(flatten, default)]
    pub children: Vec<UlContent>,
}

/// Content types that can appear inside an unordered list.
///
/// Includes text nodes to preserve whitespace between list items.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum UlContent {
    /// Text node (for whitespace between list items).
    #[facet(text)]
    Text(String),
    /// List item.
    #[facet(rename = "li")]
    Li(Li),
}

/// List item.
#[derive(Debug, Default, Facet)]
#[facet(rename = "li")]
pub struct Li {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Value (for ol).
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Description list.
#[derive(Debug, Default, Facet)]
#[facet(rename = "dl")]
pub struct Dl {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Terms and descriptions (mixed dt/dd in order).
    #[facet(flatten, default)]
    pub children: Vec<DlContent>,
}

/// Content types that can appear inside a description list.
///
/// Includes text nodes to preserve whitespace between terms and descriptions.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum DlContent {
    /// Text node (for whitespace between elements).
    #[facet(text)]
    Text(String),
    /// Description term.
    #[facet(rename = "dt")]
    Dt(Dt),
    /// Description details.
    #[facet(rename = "dd")]
    Dd(Dd),
}

/// Description term.
#[derive(Debug, Default, Facet)]
#[facet(rename = "dt")]
pub struct Dt {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Description details.
#[derive(Debug, Default, Facet)]
#[facet(rename = "dd")]
pub struct Dd {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Figure with optional caption.
#[derive(Debug, Default, Facet)]
#[facet(rename = "figure")]
pub struct Figure {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Figure caption.
    #[facet(default)]
    pub figcaption: Option<Figcaption>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Figure caption.
#[derive(Debug, Default, Facet)]
#[facet(rename = "figcaption")]
pub struct Figcaption {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Horizontal rule (thematic break).
#[derive(Debug, Default, Facet)]
#[facet(rename = "hr")]
pub struct Hr {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
}

// =============================================================================
// Text-level Semantics
// =============================================================================

/// Hyperlink.
#[derive(Debug, Default, Facet)]
#[facet(rename = "a")]
pub struct A {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// URL.
    #[facet(attribute, default)]
    pub href: Option<String>,
    /// Target browsing context.
    #[facet(attribute, default)]
    pub target: Option<String>,
    /// Relationship.
    #[facet(attribute, default)]
    pub rel: Option<String>,
    /// Download filename.
    #[facet(attribute, default)]
    pub download: Option<String>,
    /// MIME type hint.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Language of linked resource.
    #[facet(attribute, default)]
    pub hreflang: Option<String>,
    /// Referrer policy.
    #[facet(attribute, default)]
    pub referrerpolicy: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Emphasis.
#[derive(Debug, Default, Facet)]
#[facet(rename = "em")]
pub struct Em {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Strong importance.
#[derive(Debug, Default, Facet)]
#[facet(rename = "strong")]
pub struct Strong {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Small print.
#[derive(Debug, Default, Facet)]
#[facet(rename = "small")]
pub struct Small {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Strikethrough (no longer accurate).
#[derive(Debug, Default, Facet)]
#[facet(rename = "s")]
pub struct S {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Citation.
#[derive(Debug, Default, Facet)]
#[facet(rename = "cite")]
pub struct Cite {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Inline quotation.
#[derive(Debug, Default, Facet)]
#[facet(rename = "q")]
pub struct Q {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Citation URL.
    #[facet(attribute, default)]
    pub cite: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Definition term.
#[derive(Debug, Default, Facet)]
#[facet(rename = "dfn")]
pub struct Dfn {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Abbreviation.
#[derive(Debug, Default, Facet)]
#[facet(rename = "abbr")]
pub struct Abbr {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Ruby annotation (for East Asian typography).
#[derive(Debug, Default, Facet)]
#[facet(rename = "ruby")]
pub struct Ruby {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Data with machine-readable value.
#[derive(Debug, Default, Facet)]
#[facet(rename = "data")]
pub struct Data {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Machine-readable value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Time.
#[derive(Debug, Default, Facet)]
#[facet(rename = "time")]
pub struct Time {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Machine-readable datetime.
    #[facet(attribute, default)]
    pub datetime: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Code fragment.
#[derive(Debug, Default, Facet)]
#[facet(rename = "code")]
pub struct Code {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Variable.
#[derive(Debug, Default, Facet)]
#[facet(rename = "var")]
pub struct Var {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Sample output.
#[derive(Debug, Default, Facet)]
#[facet(rename = "samp")]
pub struct Samp {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Keyboard input.
#[derive(Debug, Default, Facet)]
#[facet(rename = "kbd")]
pub struct Kbd {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Subscript.
#[derive(Debug, Default, Facet)]
#[facet(rename = "sub")]
pub struct Sub {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Superscript.
#[derive(Debug, Default, Facet)]
#[facet(rename = "sup")]
pub struct Sup {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Italic.
#[derive(Debug, Default, Facet)]
#[facet(rename = "i")]
pub struct I {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Bold.
#[derive(Debug, Default, Facet)]
#[facet(rename = "b")]
pub struct B {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Underline.
#[derive(Debug, Default, Facet)]
#[facet(rename = "u")]
pub struct U {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Highlighted text.
#[derive(Debug, Default, Facet)]
#[facet(rename = "mark")]
pub struct Mark {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Bidirectional isolation.
#[derive(Debug, Default, Facet)]
#[facet(rename = "bdi")]
pub struct Bdi {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Bidirectional override.
#[derive(Debug, Default, Facet)]
#[facet(rename = "bdo")]
pub struct Bdo {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Line break.
#[derive(Debug, Default, Facet)]
#[facet(rename = "br")]
pub struct Br {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
}

/// Word break opportunity.
#[derive(Debug, Default, Facet)]
#[facet(rename = "wbr")]
pub struct Wbr {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
}

// =============================================================================
// Embedded Content
// =============================================================================

/// Image.
#[derive(Debug, Default, Facet)]
#[facet(rename = "img")]
pub struct Img {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Image URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Alternative text.
    #[facet(attribute, default)]
    pub alt: Option<String>,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// Srcset for responsive images.
    #[facet(attribute, default)]
    pub srcset: Option<String>,
    /// Sizes attribute.
    #[facet(attribute, default)]
    pub sizes: Option<String>,
    /// Loading behavior.
    #[facet(attribute, default)]
    pub loading: Option<String>,
    /// Decoding hint.
    #[facet(attribute, default)]
    pub decoding: Option<String>,
    /// Crossorigin.
    #[facet(attribute, default)]
    pub crossorigin: Option<String>,
    /// Referrer policy.
    #[facet(attribute, default)]
    pub referrerpolicy: Option<String>,
    /// Usemap reference.
    #[facet(attribute, default)]
    pub usemap: Option<String>,
    /// Whether this is a server-side image map.
    #[facet(attribute, default)]
    pub ismap: Option<String>,
}

/// Inline frame.
#[derive(Debug, Default, Facet)]
#[facet(rename = "iframe")]
pub struct Iframe {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Srcdoc content.
    #[facet(attribute, default)]
    pub srcdoc: Option<String>,
    /// Frame name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// Sandbox restrictions.
    #[facet(attribute, default)]
    pub sandbox: Option<String>,
    /// Feature policy.
    #[facet(attribute, default)]
    pub allow: Option<String>,
    /// Fullscreen allowed.
    #[facet(attribute, default)]
    pub allowfullscreen: Option<String>,
    /// Loading behavior.
    #[facet(attribute, default)]
    pub loading: Option<String>,
    /// Referrer policy.
    #[facet(attribute, default)]
    pub referrerpolicy: Option<String>,
}

/// Embedded object.
#[derive(Debug, Default, Facet)]
#[facet(rename = "object")]
pub struct Object {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Data URL.
    #[facet(attribute, default)]
    pub data: Option<String>,
    /// MIME type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// Usemap reference.
    #[facet(attribute, default)]
    pub usemap: Option<String>,
    /// Fallback content.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Video player.
#[derive(Debug, Default, Facet)]
#[facet(rename = "video")]
pub struct Video {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Video URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Poster image.
    #[facet(attribute, default)]
    pub poster: Option<String>,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// Show controls.
    #[facet(attribute, default)]
    pub controls: Option<String>,
    /// Autoplay.
    #[facet(attribute, default)]
    pub autoplay: Option<String>,
    /// Loop playback.
    #[facet(attribute, default, rename = "loop")]
    pub loop_: Option<String>,
    /// Muted by default.
    #[facet(attribute, default)]
    pub muted: Option<String>,
    /// Preload behavior.
    #[facet(attribute, default)]
    pub preload: Option<String>,
    /// Plays inline (iOS).
    #[facet(attribute, default)]
    pub playsinline: Option<String>,
    /// Crossorigin.
    #[facet(attribute, default)]
    pub crossorigin: Option<String>,
    /// Child elements (source, track elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<VideoContent>,
}

/// Content types that can appear inside a video element.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum VideoContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Source element.
    #[facet(rename = "source")]
    Source(Source),
    /// Track element.
    #[facet(rename = "track")]
    Track(Track),
}

/// Audio player.
#[derive(Debug, Default, Facet)]
#[facet(rename = "audio")]
pub struct Audio {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Audio URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Show controls.
    #[facet(attribute, default)]
    pub controls: Option<String>,
    /// Autoplay.
    #[facet(attribute, default)]
    pub autoplay: Option<String>,
    /// Loop playback.
    #[facet(attribute, default, rename = "loop")]
    pub loop_: Option<String>,
    /// Muted by default.
    #[facet(attribute, default)]
    pub muted: Option<String>,
    /// Preload behavior.
    #[facet(attribute, default)]
    pub preload: Option<String>,
    /// Crossorigin.
    #[facet(attribute, default)]
    pub crossorigin: Option<String>,
    /// Child elements (source elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<AudioContent>,
}

/// Content types that can appear inside an audio element.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum AudioContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Source element.
    #[facet(rename = "source")]
    Source(Source),
}

/// Media source.
#[derive(Debug, Default, Facet)]
#[facet(rename = "source")]
pub struct Source {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// MIME type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Srcset (for picture).
    #[facet(attribute, default)]
    pub srcset: Option<String>,
    /// Sizes.
    #[facet(attribute, default)]
    pub sizes: Option<String>,
    /// Media query.
    #[facet(attribute, default)]
    pub media: Option<String>,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
}

/// Text track for video/audio.
#[derive(Debug, Default, Facet)]
#[facet(rename = "track")]
pub struct Track {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Track kind.
    #[facet(attribute, default)]
    pub kind: Option<String>,
    /// Language.
    #[facet(attribute, default)]
    pub srclang: Option<String>,
    /// Label.
    #[facet(attribute, default)]
    pub label: Option<String>,
    /// Default track.
    #[facet(attribute, default)]
    pub default: Option<String>,
}

/// Picture element for art direction.
#[derive(Debug, Default, Facet)]
#[facet(rename = "picture")]
pub struct Picture {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (source, img elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<PictureContent>,
}

/// Content types that can appear inside a picture element.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum PictureContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Source element.
    #[facet(rename = "source")]
    Source(Source),
    /// Fallback image.
    #[facet(rename = "img")]
    Img(Img),
}

/// Canvas for graphics.
#[derive(Debug, Default, Facet)]
#[facet(rename = "canvas")]
pub struct Canvas {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// Fallback content.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// SVG root element (simplified).
#[derive(Debug, Default, Facet)]
#[facet(rename = "svg")]
pub struct Svg {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Width.
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height.
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// ViewBox.
    #[facet(attribute, default, rename = "viewBox")]
    pub view_box: Option<String>,
    /// Xmlns.
    #[facet(attribute, default)]
    pub xmlns: Option<String>,
    /// Preserve aspect ratio.
    #[facet(attribute, default, rename = "preserveAspectRatio")]
    pub preserve_aspect_ratio: Option<String>,
    /// Child elements (SVG content).
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<SvgContent>,
}

/// A custom SVG element with a dynamic tag name.
///
/// This captures any SVG element that isn't explicitly modeled, preserving
/// its tag name, attributes, and children during parse/serialize roundtrips.
#[derive(Debug, Default, Facet)]
pub struct CustomSvgElement {
    /// The tag name of the SVG element (e.g., "rect", "path", "g").
    #[facet(tag, default)]
    pub tag: String,
    /// Global attributes (id, class, style, data-*, etc.).
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<SvgContent>,
}

/// SVG content - elements and text that can appear inside an SVG.
#[derive(Debug, Facet)]
#[repr(u8)]
#[allow(clippy::large_enum_variant)] // DOM-like structures naturally have large variants
pub enum SvgContent {
    /// Text node (named to avoid collision with SVG `<text>` element).
    #[facet(text)]
    TextNode(String),
    /// Any SVG element (catch-all).
    #[facet(custom_element)]
    Element(CustomSvgElement),
}

// =============================================================================
// Table Elements
// =============================================================================

/// Table.
#[derive(Debug, Default, Facet)]
#[facet(rename = "table")]
pub struct Table {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (caption, colgroup, thead, tbody, tfoot, tr, and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<TableContent>,
}

/// Content types that can appear inside a table.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum TableContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Caption.
    #[facet(rename = "caption")]
    Caption(Caption),
    /// Column group.
    #[facet(rename = "colgroup")]
    Colgroup(Colgroup),
    /// Table head.
    #[facet(rename = "thead")]
    Thead(Thead),
    /// Table body.
    #[facet(rename = "tbody")]
    Tbody(Tbody),
    /// Table foot.
    #[facet(rename = "tfoot")]
    Tfoot(Tfoot),
    /// Table row.
    #[facet(rename = "tr")]
    Tr(Tr),
}

/// Table caption.
#[derive(Debug, Default, Facet)]
#[facet(rename = "caption")]
pub struct Caption {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Column group.
#[derive(Debug, Default, Facet)]
#[facet(rename = "colgroup")]
pub struct Colgroup {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Number of columns spanned.
    #[facet(attribute, default)]
    pub span: Option<String>,
    /// Child elements (col elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<ColgroupContent>,
}

/// Content types that can appear inside a colgroup.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum ColgroupContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Column definition.
    #[facet(rename = "col")]
    Col(Col),
}

/// Table column.
#[derive(Debug, Default, Facet)]
#[facet(rename = "col")]
pub struct Col {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Number of columns spanned.
    #[facet(attribute, default)]
    pub span: Option<String>,
}

/// Table head.
#[derive(Debug, Default, Facet)]
#[facet(rename = "thead")]
pub struct Thead {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (tr elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<TableSectionContent>,
}

/// Table body.
#[derive(Debug, Default, Facet)]
#[facet(rename = "tbody")]
pub struct Tbody {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (tr elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<TableSectionContent>,
}

/// Table foot.
#[derive(Debug, Default, Facet)]
#[facet(rename = "tfoot")]
pub struct Tfoot {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (tr elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<TableSectionContent>,
}

/// Content types that can appear inside thead, tbody, or tfoot.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum TableSectionContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Table row.
    #[facet(rename = "tr")]
    Tr(Tr),
}

/// Table row.
#[derive(Debug, Default, Facet)]
#[facet(rename = "tr")]
pub struct Tr {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (th, td elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<TrContent>,
}

/// Content types that can appear inside a table row.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum TrContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Header cell.
    #[facet(rename = "th")]
    Th(Th),
    /// Data cell.
    #[facet(rename = "td")]
    Td(Td),
}

/// Table header cell.
#[derive(Debug, Default, Facet)]
#[facet(rename = "th")]
pub struct Th {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Number of columns spanned.
    #[facet(attribute, default)]
    pub colspan: Option<String>,
    /// Number of rows spanned.
    #[facet(attribute, default)]
    pub rowspan: Option<String>,
    /// Header scope.
    #[facet(attribute, default)]
    pub scope: Option<String>,
    /// Headers this cell relates to.
    #[facet(attribute, default)]
    pub headers: Option<String>,
    /// Abbreviation.
    #[facet(attribute, default)]
    pub abbr: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Table data cell.
#[derive(Debug, Default, Facet)]
#[facet(rename = "td")]
pub struct Td {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Number of columns spanned.
    #[facet(attribute, default)]
    pub colspan: Option<String>,
    /// Number of rows spanned.
    #[facet(attribute, default)]
    pub rowspan: Option<String>,
    /// Headers this cell relates to.
    #[facet(attribute, default)]
    pub headers: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

// =============================================================================
// Form Elements
// =============================================================================

/// Form.
#[derive(Debug, Default, Facet)]
#[facet(rename = "form")]
pub struct Form {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Form action URL.
    #[facet(attribute, default)]
    pub action: Option<String>,
    /// HTTP method.
    #[facet(attribute, default)]
    pub method: Option<String>,
    /// Encoding type.
    #[facet(attribute, default)]
    pub enctype: Option<String>,
    /// Target.
    #[facet(attribute, default)]
    pub target: Option<String>,
    /// Form name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Autocomplete.
    #[facet(attribute, default)]
    pub autocomplete: Option<String>,
    /// Disable validation.
    #[facet(attribute, default)]
    pub novalidate: Option<String>,
    /// Accept-charset.
    #[facet(attribute, default, rename = "accept-charset")]
    pub accept_charset: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Input control.
#[derive(Debug, Default, Facet)]
#[facet(rename = "input")]
pub struct Input {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Input type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Placeholder.
    #[facet(attribute, default)]
    pub placeholder: Option<String>,
    /// Required.
    #[facet(attribute, default)]
    pub required: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Readonly.
    #[facet(attribute, default)]
    pub readonly: Option<String>,
    /// Checked (for checkboxes/radios).
    #[facet(attribute, default)]
    pub checked: Option<String>,
    /// Autocomplete.
    #[facet(attribute, default)]
    pub autocomplete: Option<String>,
    /// Autofocus.
    #[facet(attribute, default)]
    pub autofocus: Option<String>,
    /// Min value.
    #[facet(attribute, default)]
    pub min: Option<String>,
    /// Max value.
    #[facet(attribute, default)]
    pub max: Option<String>,
    /// Step.
    #[facet(attribute, default)]
    pub step: Option<String>,
    /// Pattern.
    #[facet(attribute, default)]
    pub pattern: Option<String>,
    /// Size.
    #[facet(attribute, default)]
    pub size: Option<String>,
    /// Maxlength.
    #[facet(attribute, default)]
    pub maxlength: Option<String>,
    /// Minlength.
    #[facet(attribute, default)]
    pub minlength: Option<String>,
    /// Multiple values allowed.
    #[facet(attribute, default)]
    pub multiple: Option<String>,
    /// Accept (for file inputs).
    #[facet(attribute, default)]
    pub accept: Option<String>,
    /// Alt text (for image inputs).
    #[facet(attribute, default)]
    pub alt: Option<String>,
    /// Src (for image inputs).
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// Width (for image inputs).
    #[facet(attribute, default)]
    pub width: Option<String>,
    /// Height (for image inputs).
    #[facet(attribute, default)]
    pub height: Option<String>,
    /// List datalist reference.
    #[facet(attribute, default)]
    pub list: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Form action override.
    #[facet(attribute, default)]
    pub formaction: Option<String>,
    /// Form method override.
    #[facet(attribute, default)]
    pub formmethod: Option<String>,
    /// Form enctype override.
    #[facet(attribute, default)]
    pub formenctype: Option<String>,
    /// Form target override.
    #[facet(attribute, default)]
    pub formtarget: Option<String>,
    /// Form novalidate override.
    #[facet(attribute, default)]
    pub formnovalidate: Option<String>,
}

/// Button.
#[derive(Debug, Default, Facet)]
#[facet(rename = "button")]
pub struct Button {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Button type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Autofocus.
    #[facet(attribute, default)]
    pub autofocus: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Form action override.
    #[facet(attribute, default)]
    pub formaction: Option<String>,
    /// Form method override.
    #[facet(attribute, default)]
    pub formmethod: Option<String>,
    /// Form enctype override.
    #[facet(attribute, default)]
    pub formenctype: Option<String>,
    /// Form target override.
    #[facet(attribute, default)]
    pub formtarget: Option<String>,
    /// Form novalidate override.
    #[facet(attribute, default)]
    pub formnovalidate: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Select dropdown.
#[derive(Debug, Default, Facet)]
#[facet(rename = "select")]
pub struct Select {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Multiple selection.
    #[facet(attribute, default)]
    pub multiple: Option<String>,
    /// Size (visible options).
    #[facet(attribute, default)]
    pub size: Option<String>,
    /// Required.
    #[facet(attribute, default)]
    pub required: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Autofocus.
    #[facet(attribute, default)]
    pub autofocus: Option<String>,
    /// Autocomplete.
    #[facet(attribute, default)]
    pub autocomplete: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Child elements (option, optgroup, and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<SelectContent>,
}

/// Content types that can appear inside a select.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum SelectContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Option.
    #[facet(rename = "option")]
    Option(OptionElement),
    /// Option group.
    #[facet(rename = "optgroup")]
    Optgroup(Optgroup),
}

/// Option in a select.
#[derive(Debug, Default, Facet)]
#[facet(rename = "option")]
pub struct OptionElement {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Selected.
    #[facet(attribute, default)]
    pub selected: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Label.
    #[facet(attribute, default)]
    pub label: Option<String>,
    /// Text content.
    #[facet(text, default)]
    pub text: String,
}

/// Option group.
#[derive(Debug, Default, Facet)]
#[facet(rename = "optgroup")]
pub struct Optgroup {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Label.
    #[facet(attribute, default)]
    pub label: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Child elements (option elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<OptgroupContent>,
}

/// Content types that can appear inside an optgroup.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum OptgroupContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Option.
    #[facet(rename = "option")]
    Option(OptionElement),
}

/// Textarea.
#[derive(Debug, Default, Facet)]
#[facet(rename = "textarea")]
pub struct Textarea {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Rows.
    #[facet(attribute, default)]
    pub rows: Option<String>,
    /// Cols.
    #[facet(attribute, default)]
    pub cols: Option<String>,
    /// Placeholder.
    #[facet(attribute, default)]
    pub placeholder: Option<String>,
    /// Required.
    #[facet(attribute, default)]
    pub required: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Readonly.
    #[facet(attribute, default)]
    pub readonly: Option<String>,
    /// Autofocus.
    #[facet(attribute, default)]
    pub autofocus: Option<String>,
    /// Autocomplete.
    #[facet(attribute, default)]
    pub autocomplete: Option<String>,
    /// Maxlength.
    #[facet(attribute, default)]
    pub maxlength: Option<String>,
    /// Minlength.
    #[facet(attribute, default)]
    pub minlength: Option<String>,
    /// Wrap.
    #[facet(attribute, default)]
    pub wrap: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Text content.
    #[facet(text, default)]
    pub text: String,
}

/// Form label.
#[derive(Debug, Default, Facet)]
#[facet(rename = "label")]
pub struct Label {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Associated control ID.
    #[facet(attribute, default, rename = "for")]
    pub for_: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Fieldset grouping.
#[derive(Debug, Default, Facet)]
#[facet(rename = "fieldset")]
pub struct Fieldset {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Disabled.
    #[facet(attribute, default)]
    pub disabled: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Legend.
    #[facet(default)]
    pub legend: Option<Legend>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Fieldset legend.
#[derive(Debug, Default, Facet)]
#[facet(rename = "legend")]
pub struct Legend {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Datalist.
#[derive(Debug, Default, Facet)]
#[facet(rename = "datalist")]
pub struct Datalist {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements (option elements and whitespace).
    #[facet(flatten, default)]
    pub children: Vec<DatalistContent>,
}

/// Content types that can appear inside a datalist.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum DatalistContent {
    /// Text node (for whitespace).
    #[facet(text)]
    Text(String),
    /// Option.
    #[facet(rename = "option")]
    Option(OptionElement),
}

/// Output.
#[derive(Debug, Default, Facet)]
#[facet(rename = "output")]
pub struct Output {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Associated controls.
    #[facet(attribute, default, rename = "for")]
    pub for_: Option<String>,
    /// Name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Form override.
    #[facet(attribute, default)]
    pub form: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Progress indicator.
#[derive(Debug, Default, Facet)]
#[facet(rename = "progress")]
pub struct Progress {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Current value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Maximum value.
    #[facet(attribute, default)]
    pub max: Option<String>,
    /// Fallback content.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Meter/gauge.
#[derive(Debug, Default, Facet)]
#[facet(rename = "meter")]
pub struct Meter {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Current value.
    #[facet(attribute, default)]
    pub value: Option<String>,
    /// Minimum value.
    #[facet(attribute, default)]
    pub min: Option<String>,
    /// Maximum value.
    #[facet(attribute, default)]
    pub max: Option<String>,
    /// Low threshold.
    #[facet(attribute, default)]
    pub low: Option<String>,
    /// High threshold.
    #[facet(attribute, default)]
    pub high: Option<String>,
    /// Optimum value.
    #[facet(attribute, default)]
    pub optimum: Option<String>,
    /// Fallback content.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

// =============================================================================
// Interactive Elements
// =============================================================================

/// Details disclosure widget.
#[derive(Debug, Default, Facet)]
#[facet(rename = "details")]
pub struct Details {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Open state.
    #[facet(attribute, default)]
    pub open: Option<String>,
    /// Summary.
    #[facet(default)]
    pub summary: Option<Summary>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Details summary.
#[derive(Debug, Default, Facet)]
#[facet(rename = "summary")]
pub struct Summary {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

/// Dialog box.
#[derive(Debug, Default, Facet)]
#[facet(rename = "dialog")]
pub struct Dialog {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Open state.
    #[facet(attribute, default)]
    pub open: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

// =============================================================================
// Scripting Elements
// =============================================================================

/// Script.
#[derive(Debug, Default, Facet)]
#[facet(rename = "script")]
pub struct Script {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Script URL.
    #[facet(attribute, default)]
    pub src: Option<String>,
    /// MIME type.
    #[facet(attribute, default, rename = "type")]
    pub type_: Option<String>,
    /// Async loading.
    #[facet(attribute, default, rename = "async")]
    pub async_: Option<String>,
    /// Defer loading.
    #[facet(attribute, default)]
    pub defer: Option<String>,
    /// Crossorigin.
    #[facet(attribute, default)]
    pub crossorigin: Option<String>,
    /// Integrity hash.
    #[facet(attribute, default)]
    pub integrity: Option<String>,
    /// Referrer policy.
    #[facet(attribute, default)]
    pub referrerpolicy: Option<String>,
    /// Nomodule flag.
    #[facet(attribute, default)]
    pub nomodule: Option<String>,
    /// Inline script content.
    #[facet(text, default)]
    pub text: String,
}

/// Noscript fallback.
#[derive(Debug, Default, Facet)]
#[facet(rename = "noscript")]
pub struct Noscript {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Template.
#[derive(Debug, Default, Facet)]
#[facet(rename = "template")]
pub struct Template {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// Slot for web components.
#[derive(Debug, Default, Facet)]
#[facet(rename = "slot")]
pub struct Slot {
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Slot name.
    #[facet(attribute, default)]
    pub name: Option<String>,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

// =============================================================================
// Custom Elements
// =============================================================================

/// A custom HTML element with a dynamic tag name.
///
/// This type is used as a catch-all for unknown elements during HTML parsing.
/// Custom elements (like `<a-k>`, `<a-f>` from arborium syntax highlighting)
/// are preserved with their tag name, attributes, and children.
///
/// # Example
///
/// ```ignore
/// // Input: <a-k>fn</a-k>
/// // Parses as:
/// CustomElement {
///     tag: "a-k".to_string(),
///     attrs: GlobalAttrs::default(),
///     children: vec![FlowContent::Text("fn".to_string())],
/// }
/// ```
#[derive(Debug, Default, Facet)]
pub struct CustomElement {
    /// The tag name of the custom element (e.g., "a-k", "my-component").
    ///
    /// This field is marked with `#[facet(tag)]` to indicate it should
    /// receive the element's tag name during deserialization.
    #[facet(tag, default)]
    pub tag: String,
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<FlowContent>,
}

/// A custom phrasing element with a dynamic tag name.
///
/// Similar to [`CustomElement`] but for inline/phrasing content contexts.
/// This allows custom elements to appear inside paragraphs, spans, etc.
#[derive(Debug, Default, Facet)]
pub struct CustomPhrasingElement {
    /// The tag name of the custom element.
    #[facet(tag, default)]
    pub tag: String,
    /// Global attributes.
    #[facet(flatten, default)]
    pub attrs: GlobalAttrs,
    /// Child elements.
    #[facet(flatten, default)]
    #[facet(recursive_type)]
    pub children: Vec<PhrasingContent>,
}

// =============================================================================
// Content Categories (Enums for mixed content)
// =============================================================================

/// Metadata content - elements that can appear in `<head>`.
#[derive(Debug, Facet)]
#[repr(u8)]
pub enum MetadataContent {
    /// Text node (for whitespace between elements).
    #[facet(text)]
    Text(String),
    /// Document title.
    #[facet(rename = "title")]
    Title(Title),
    /// Base URL element.
    #[facet(rename = "base")]
    Base(Base),
    /// Linked resources (stylesheets, icons, etc.).
    #[facet(rename = "link")]
    Link(Link),
    /// Metadata elements.
    #[facet(rename = "meta")]
    Meta(Meta),
    /// Inline styles.
    #[facet(rename = "style")]
    Style(Style),
    /// Scripts.
    #[facet(rename = "script")]
    Script(Script),
    /// Noscript element.
    #[facet(rename = "noscript")]
    Noscript(Noscript),
    /// Template element.
    #[facet(rename = "template")]
    Template(Template),
}

/// Flow content - most block and inline elements.
#[derive(Debug, Facet)]
#[repr(u8)]
#[allow(clippy::large_enum_variant)] // DOM-like structures naturally have large variants
pub enum FlowContent {
    /// Text node (for mixed content).
    #[facet(text)]
    Text(String),

    // Sections
    /// Header element.
    #[facet(rename = "header")]
    Header(Header),
    /// Footer element.
    #[facet(rename = "footer")]
    Footer(Footer),
    /// Main element.
    #[facet(rename = "main")]
    Main(Main),
    /// Article element.
    #[facet(rename = "article")]
    Article(Article),
    /// Section element.
    #[facet(rename = "section")]
    Section(Section),
    /// Nav element.
    #[facet(rename = "nav")]
    Nav(Nav),
    /// Aside element.
    #[facet(rename = "aside")]
    Aside(Aside),
    /// Address element.
    #[facet(rename = "address")]
    Address(Address),

    // Headings
    /// H1 element.
    #[facet(rename = "h1")]
    H1(H1),
    /// H2 element.
    #[facet(rename = "h2")]
    H2(H2),
    /// H3 element.
    #[facet(rename = "h3")]
    H3(H3),
    /// H4 element.
    #[facet(rename = "h4")]
    H4(H4),
    /// H5 element.
    #[facet(rename = "h5")]
    H5(H5),
    /// H6 element.
    #[facet(rename = "h6")]
    H6(H6),

    // Grouping
    /// P element.
    #[facet(rename = "p")]
    P(P),
    /// Div element.
    #[facet(rename = "div")]
    Div(Div),
    /// Pre element.
    #[facet(rename = "pre")]
    Pre(Pre),
    /// Blockquote element.
    #[facet(rename = "blockquote")]
    Blockquote(Blockquote),
    /// Ol element.
    #[facet(rename = "ol")]
    Ol(Ol),
    /// Ul element.
    #[facet(rename = "ul")]
    Ul(Ul),
    /// Dl element.
    #[facet(rename = "dl")]
    Dl(Dl),
    /// Figure element.
    #[facet(rename = "figure")]
    Figure(Figure),
    /// Hr element.
    #[facet(rename = "hr")]
    Hr(Hr),

    // Phrasing (inline)
    /// A element.
    #[facet(rename = "a")]
    A(A),
    /// Span element.
    #[facet(rename = "span")]
    Span(Span),
    /// Em element.
    #[facet(rename = "em")]
    Em(Em),
    /// Strong element.
    #[facet(rename = "strong")]
    Strong(Strong),
    /// Code element.
    #[facet(rename = "code")]
    Code(Code),
    /// Img element.
    #[facet(rename = "img")]
    Img(Img),
    /// Br element.
    #[facet(rename = "br")]
    Br(Br),

    // Tables
    /// Table element.
    #[facet(rename = "table")]
    Table(Table),

    // Forms
    /// Form element.
    #[facet(rename = "form")]
    Form(Form),
    /// Input element.
    #[facet(rename = "input")]
    Input(Input),
    /// Button element.
    #[facet(rename = "button")]
    Button(Button),
    /// Select element.
    #[facet(rename = "select")]
    Select(Select),
    /// Textarea element.
    #[facet(rename = "textarea")]
    Textarea(Textarea),
    /// Label element.
    #[facet(rename = "label")]
    Label(Label),
    /// Fieldset element.
    #[facet(rename = "fieldset")]
    Fieldset(Fieldset),

    // Interactive
    /// Details element.
    #[facet(rename = "details")]
    Details(Details),
    /// Dialog element.
    #[facet(rename = "dialog")]
    Dialog(Dialog),

    // Embedded
    /// Iframe element.
    #[facet(rename = "iframe")]
    Iframe(Iframe),
    /// Video element.
    #[facet(rename = "video")]
    Video(Video),
    /// Audio element.
    #[facet(rename = "audio")]
    Audio(Audio),
    /// Picture element.
    #[facet(rename = "picture")]
    Picture(Picture),
    /// Canvas element.
    #[facet(rename = "canvas")]
    Canvas(Canvas),
    /// Svg element.
    #[facet(rename = "svg")]
    Svg(Svg),

    // Scripting
    /// Script element.
    #[facet(rename = "script")]
    Script(Script),
    /// Noscript element.
    #[facet(rename = "noscript")]
    Noscript(Noscript),
    /// Template element.
    #[facet(rename = "template")]
    Template(Template),

    // Custom elements (catch-all for unknown elements)
    /// Custom element (catch-all for unknown elements like `<a-k>`, `<my-component>`).
    #[facet(custom_element)]
    Custom(CustomElement),
}

/// Phrasing content - inline elements and text.
#[derive(Debug, Facet)]
#[repr(u8)]
#[allow(clippy::large_enum_variant)] // DOM-like structures naturally have large variants
pub enum PhrasingContent {
    /// Text node (for mixed content).
    #[facet(text)]
    Text(String),
    /// A element.
    #[facet(rename = "a")]
    A(A),
    /// Span element.
    #[facet(rename = "span")]
    Span(Span),
    /// Em element.
    #[facet(rename = "em")]
    Em(Em),
    /// Strong element.
    #[facet(rename = "strong")]
    Strong(Strong),
    /// Small element.
    #[facet(rename = "small")]
    Small(Small),
    /// S element.
    #[facet(rename = "s")]
    S(S),
    /// Cite element.
    #[facet(rename = "cite")]
    Cite(Cite),
    /// Q element.
    #[facet(rename = "q")]
    Q(Q),
    /// Dfn element.
    #[facet(rename = "dfn")]
    Dfn(Dfn),
    /// Abbr element.
    #[facet(rename = "abbr")]
    Abbr(Abbr),
    /// Data element.
    #[facet(rename = "data")]
    Data(Data),
    /// Time element.
    #[facet(rename = "time")]
    Time(Time),
    /// Code element.
    #[facet(rename = "code")]
    Code(Code),
    /// Var element.
    #[facet(rename = "var")]
    Var(Var),
    /// Samp element.
    #[facet(rename = "samp")]
    Samp(Samp),
    /// Kbd element.
    #[facet(rename = "kbd")]
    Kbd(Kbd),
    /// Sub element.
    #[facet(rename = "sub")]
    Sub(Sub),
    /// Sup element.
    #[facet(rename = "sup")]
    Sup(Sup),
    /// I element.
    #[facet(rename = "i")]
    I(I),
    /// B element.
    #[facet(rename = "b")]
    B(B),
    /// U element.
    #[facet(rename = "u")]
    U(U),
    /// Mark element.
    #[facet(rename = "mark")]
    Mark(Mark),
    /// Bdi element.
    #[facet(rename = "bdi")]
    Bdi(Bdi),
    /// Bdo element.
    #[facet(rename = "bdo")]
    Bdo(Bdo),
    /// Br element.
    #[facet(rename = "br")]
    Br(Br),
    /// Wbr element.
    #[facet(rename = "wbr")]
    Wbr(Wbr),
    /// Img element.
    #[facet(rename = "img")]
    Img(Img),
    /// Input element.
    #[facet(rename = "input")]
    Input(Input),
    /// Button element.
    #[facet(rename = "button")]
    Button(Button),
    /// Select element.
    #[facet(rename = "select")]
    Select(Select),
    /// Textarea element.
    #[facet(rename = "textarea")]
    Textarea(Textarea),
    /// Label element.
    #[facet(rename = "label")]
    Label(Label),
    /// Output element.
    #[facet(rename = "output")]
    Output(Output),
    /// Progress element.
    #[facet(rename = "progress")]
    Progress(Progress),
    /// Meter element.
    #[facet(rename = "meter")]
    Meter(Meter),
    /// Script element.
    #[facet(rename = "script")]
    Script(Script),

    // Custom elements (catch-all for unknown elements)
    /// Custom element (catch-all for unknown elements like `<a-k>`, `<my-component>`).
    #[facet(custom_element)]
    Custom(CustomPhrasingElement),
}
