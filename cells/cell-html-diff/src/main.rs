//! Dodeca HTML diff cell (cell-html-diff)
//!
//! This cell handles HTML DOM diffing for live reload using facet-format-html
//! for parsing and facet-diff for computing structural differences.

use cell_html_diff_proto::{DiffInput, DiffResult, HtmlDiffResult, HtmlDiffer, HtmlDifferServer};
use facet::Facet;
use facet_diff::{EditOp, tree_diff};
use facet_format_html as html;
use facet_format_xml as xml;

// Re-export protocol types
pub use dodeca_protocol::{NodePath, Patch};

// ============================================================================
// HTML Document Model for diffing
// ============================================================================

/// An HTML document with head and body sections.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "html", pod)]
struct HtmlDocument {
    #[facet(xml::element, default)]
    head: Option<Head>,
    #[facet(xml::element, default)]
    body: Option<Body>,
}

/// The `<head>` section of an HTML document.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "head", pod)]
struct Head {
    #[facet(xml::element, default)]
    title: Option<Title>,
    #[facet(xml::elements, default)]
    meta: Vec<Meta>,
    #[facet(xml::elements, default)]
    link: Vec<Link>,
    #[facet(xml::elements, default)]
    style: Vec<Style>,
    #[facet(xml::elements, default)]
    script: Vec<Script>,
}

/// A `<title>` element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "title", pod)]
struct Title {
    #[facet(xml::text, default)]
    text: String,
}

/// A `<meta>` element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "meta", pod)]
struct Meta {
    #[facet(xml::attribute, default)]
    name: Option<String>,
    #[facet(xml::attribute, default)]
    content: Option<String>,
    #[facet(xml::attribute, default)]
    charset: Option<String>,
    #[facet(xml::attribute, default, rename = "http-equiv")]
    http_equiv: Option<String>,
    #[facet(xml::attribute, default)]
    property: Option<String>,
}

/// A `<link>` element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "link", pod)]
struct Link {
    #[facet(xml::attribute, default)]
    href: Option<String>,
    #[facet(xml::attribute, default)]
    rel: Option<String>,
    #[facet(xml::attribute, default, rename = "type")]
    type_: Option<String>,
}

/// A `<style>` element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "style", pod)]
struct Style {
    #[facet(xml::text, default)]
    text: String,
}

/// A `<script>` element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "script", pod)]
struct Script {
    #[facet(xml::attribute, default)]
    src: Option<String>,
    #[facet(xml::attribute, default, rename = "type")]
    type_: Option<String>,
    #[facet(xml::attribute, default, rename = "async")]
    async_: Option<String>,
    #[facet(xml::attribute, default)]
    defer: Option<String>,
    #[facet(xml::text, default)]
    text: String,
}

/// The `<body>` section of an HTML document.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "body", pod)]
struct Body {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::elements, default)]
    children: Vec<BodyElement>,
}

/// Elements that can appear in the body.
/// This is a subset of HTML elements commonly used in web pages.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
#[repr(u8)]
enum BodyElement {
    // Sections
    #[facet(rename = "header")]
    Header(Container),
    #[facet(rename = "footer")]
    Footer(Container),
    #[facet(rename = "main")]
    Main(Container),
    #[facet(rename = "article")]
    Article(Container),
    #[facet(rename = "section")]
    Section(Container),
    #[facet(rename = "nav")]
    Nav(Container),
    #[facet(rename = "aside")]
    Aside(Container),

    // Headings
    #[facet(rename = "h1")]
    H1(TextElement),
    #[facet(rename = "h2")]
    H2(TextElement),
    #[facet(rename = "h3")]
    H3(TextElement),
    #[facet(rename = "h4")]
    H4(TextElement),
    #[facet(rename = "h5")]
    H5(TextElement),
    #[facet(rename = "h6")]
    H6(TextElement),

    // Block elements
    #[facet(rename = "div")]
    Div(Container),
    #[facet(rename = "p")]
    P(TextElement),
    #[facet(rename = "pre")]
    Pre(TextElement),
    #[facet(rename = "blockquote")]
    Blockquote(Container),

    // Lists
    #[facet(rename = "ul")]
    Ul(ListContainer),
    #[facet(rename = "ol")]
    Ol(ListContainer),

    // Inline elements
    #[facet(rename = "span")]
    Span(TextElement),
    #[facet(rename = "a")]
    A(Anchor),
    #[facet(rename = "strong")]
    Strong(TextElement),
    #[facet(rename = "em")]
    Em(TextElement),
    #[facet(rename = "code")]
    Code(TextElement),

    // Media
    #[facet(rename = "img")]
    Img(Image),

    // Tables
    #[facet(rename = "table")]
    Table(TableElement),

    // Forms
    #[facet(rename = "form")]
    Form(Container),
    #[facet(rename = "input")]
    Input(InputElement),
    #[facet(rename = "button")]
    Button(TextElement),
    #[facet(rename = "textarea")]
    Textarea(TextElement),
    #[facet(rename = "select")]
    Select(SelectElement),
    #[facet(rename = "label")]
    Label(TextElement),

    // Other
    #[facet(rename = "hr")]
    Hr(EmptyElement),
    #[facet(rename = "br")]
    Br(EmptyElement),
    #[facet(rename = "script")]
    Script(Script),
}

/// A container element with children.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct Container {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::elements, default)]
    children: Vec<BodyElement>,
    #[facet(xml::text, default)]
    text: String,
}

/// A text element (can contain text and inline children).
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct TextElement {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::text, default)]
    text: String,
}

/// A list container (ul/ol).
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct ListContainer {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::elements, default)]
    items: Vec<ListItem>,
}

/// A list item.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "li", pod)]
struct ListItem {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::elements, default)]
    children: Vec<BodyElement>,
    #[facet(xml::text, default)]
    text: String,
}

/// An anchor element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct Anchor {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::attribute, default)]
    href: Option<String>,
    #[facet(xml::attribute, default)]
    target: Option<String>,
    #[facet(xml::text, default)]
    text: String,
}

/// An image element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct Image {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::attribute, default)]
    src: Option<String>,
    #[facet(xml::attribute, default)]
    alt: Option<String>,
    #[facet(xml::attribute, default)]
    width: Option<String>,
    #[facet(xml::attribute, default)]
    height: Option<String>,
}

/// A table element (simplified).
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct TableElement {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::elements, default)]
    children: Vec<TableChild>,
}

/// Table child elements.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
#[repr(u8)]
enum TableChild {
    #[facet(rename = "thead")]
    Thead(TableSection),
    #[facet(rename = "tbody")]
    Tbody(TableSection),
    #[facet(rename = "tfoot")]
    Tfoot(TableSection),
    #[facet(rename = "tr")]
    Tr(TableRow),
}

/// A table section (thead/tbody/tfoot).
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct TableSection {
    #[facet(xml::elements, default)]
    rows: Vec<TableRow>,
}

/// A table row.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "tr", pod)]
struct TableRow {
    #[facet(xml::elements, default)]
    cells: Vec<TableCell>,
}

/// A table cell.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
#[repr(u8)]
enum TableCell {
    #[facet(rename = "th")]
    Th(TextElement),
    #[facet(rename = "td")]
    Td(TextElement),
}

/// An input element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct InputElement {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::attribute, default, rename = "type")]
    type_: Option<String>,
    #[facet(xml::attribute, default)]
    name: Option<String>,
    #[facet(xml::attribute, default)]
    value: Option<String>,
    #[facet(xml::attribute, default)]
    placeholder: Option<String>,
}

/// A select element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct SelectElement {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
    #[facet(xml::attribute, default)]
    name: Option<String>,
    #[facet(xml::elements, default)]
    options: Vec<OptionElement>,
}

/// An option element.
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(rename = "option", pod)]
struct OptionElement {
    #[facet(xml::attribute, default)]
    value: Option<String>,
    #[facet(xml::attribute, default)]
    selected: Option<String>,
    #[facet(xml::text, default)]
    text: String,
}

/// An empty element (hr, br, etc.).
#[derive(Debug, Clone, Facet, PartialEq)]
#[facet(pod)]
struct EmptyElement {
    #[facet(xml::attribute, default)]
    id: Option<String>,
    #[facet(xml::attribute, default)]
    class: Option<String>,
}

// ============================================================================
// HTML Differ Implementation
// ============================================================================

/// HTML differ implementation using facet-format-html and facet-diff.
pub struct HtmlDifferImpl;

impl HtmlDiffer for HtmlDifferImpl {
    async fn diff_html(&self, input: DiffInput) -> HtmlDiffResult {
        // Parse both HTML documents
        let old_doc: HtmlDocument = match html::from_str(&input.old_html) {
            Ok(doc) => doc,
            Err(e) => {
                return HtmlDiffResult::Error {
                    message: format!("Failed to parse old HTML: {}", e),
                };
            }
        };

        let new_doc: HtmlDocument = match html::from_str(&input.new_html) {
            Ok(doc) => doc,
            Err(e) => {
                return HtmlDiffResult::Error {
                    message: format!("Failed to parse new HTML: {}", e),
                };
            }
        };

        // Compute the tree diff
        let edit_ops = tree_diff(&old_doc, &new_doc);

        // Convert edit operations to patches
        let (patches, nodes_compared, nodes_skipped) =
            convert_edit_ops_to_patches(&edit_ops, &old_doc, &new_doc);

        HtmlDiffResult::Success {
            result: DiffResult {
                patches,
                nodes_compared,
                nodes_skipped,
            },
        }
    }
}

/// Convert facet-diff EditOps to browser DOM Patches.
///
/// The facet-diff paths look like: `body.children.[2].::ul.[0].items.[2]`
/// We need to extract positional indices for the browser DOM.
fn convert_edit_ops_to_patches(
    edit_ops: &[EditOp],
    _old_doc: &HtmlDocument,
    new_doc: &HtmlDocument,
) -> (Vec<Patch>, usize, usize) {
    let mut patches = Vec::new();
    let mut nodes_compared = 0;
    let mut nodes_skipped = 0;

    // Group operations by their base path to avoid redundant patches
    // For now, we use a simpler approach: if the body changed, replace it
    // This is a pragmatic starting point that works well for most cases

    let mut body_changed = false;
    let mut head_changed = false;

    for op in edit_ops {
        nodes_compared += 1;

        match op {
            EditOp::Update { path, .. } | EditOp::Insert { path, .. } => {
                let path_str = path.to_string();
                if path_str.starts_with("body") || path_str.is_empty() {
                    body_changed = true;
                }
                if path_str.starts_with("head") {
                    head_changed = true;
                }
            }
            EditOp::Delete { path, .. } => {
                let path_str = path.to_string();
                if path_str.starts_with("body") || path_str.is_empty() {
                    body_changed = true;
                }
                if path_str.starts_with("head") {
                    head_changed = true;
                }
            }
            EditOp::Move { new_path, .. } => {
                let path_str = new_path.to_string();
                if path_str.starts_with("body") || path_str.is_empty() {
                    body_changed = true;
                }
                if path_str.starts_with("head") {
                    head_changed = true;
                }
            }
            #[allow(unreachable_patterns)]
            _ => {
                nodes_skipped += 1;
            }
        }
    }

    // If body changed, generate a replace patch for the body
    if body_changed && let Some(body) = &new_doc.body {
        // Serialize the new body to HTML
        let body_html = serialize_body(body);
        patches.push(Patch::Replace {
            path: NodePath(vec![]), // Empty path = body element
            html: body_html,
        });
    }

    // Note: head changes typically require a full page reload
    // but we track them for stats
    if head_changed {
        nodes_compared += 1;
    }

    (patches, nodes_compared, nodes_skipped)
}

/// Serialize a Body element back to HTML string.
fn serialize_body(body: &Body) -> String {
    let mut html = String::new();

    // Build body opening tag with attributes
    html.push_str("<body");
    if let Some(id) = &body.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &body.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');

    // Serialize children
    for child in &body.children {
        serialize_element(&mut html, child);
    }

    html.push_str("</body>");
    html
}

/// Serialize a BodyElement to HTML.
fn serialize_element(html: &mut String, elem: &BodyElement) {
    match elem {
        BodyElement::Header(c) => serialize_container(html, "header", c),
        BodyElement::Footer(c) => serialize_container(html, "footer", c),
        BodyElement::Main(c) => serialize_container(html, "main", c),
        BodyElement::Article(c) => serialize_container(html, "article", c),
        BodyElement::Section(c) => serialize_container(html, "section", c),
        BodyElement::Nav(c) => serialize_container(html, "nav", c),
        BodyElement::Aside(c) => serialize_container(html, "aside", c),
        BodyElement::H1(t) => serialize_text_element(html, "h1", t),
        BodyElement::H2(t) => serialize_text_element(html, "h2", t),
        BodyElement::H3(t) => serialize_text_element(html, "h3", t),
        BodyElement::H4(t) => serialize_text_element(html, "h4", t),
        BodyElement::H5(t) => serialize_text_element(html, "h5", t),
        BodyElement::H6(t) => serialize_text_element(html, "h6", t),
        BodyElement::Div(c) => serialize_container(html, "div", c),
        BodyElement::P(t) => serialize_text_element(html, "p", t),
        BodyElement::Pre(t) => serialize_text_element(html, "pre", t),
        BodyElement::Blockquote(c) => serialize_container(html, "blockquote", c),
        BodyElement::Ul(l) => serialize_list(html, "ul", l),
        BodyElement::Ol(l) => serialize_list(html, "ol", l),
        BodyElement::Span(t) => serialize_text_element(html, "span", t),
        BodyElement::A(a) => serialize_anchor(html, a),
        BodyElement::Strong(t) => serialize_text_element(html, "strong", t),
        BodyElement::Em(t) => serialize_text_element(html, "em", t),
        BodyElement::Code(t) => serialize_text_element(html, "code", t),
        BodyElement::Img(img) => serialize_image(html, img),
        BodyElement::Table(t) => serialize_table(html, t),
        BodyElement::Form(c) => serialize_container(html, "form", c),
        BodyElement::Input(i) => serialize_input(html, i),
        BodyElement::Button(t) => serialize_text_element(html, "button", t),
        BodyElement::Textarea(t) => serialize_text_element(html, "textarea", t),
        BodyElement::Select(s) => serialize_select(html, s),
        BodyElement::Label(t) => serialize_text_element(html, "label", t),
        BodyElement::Hr(_) => html.push_str("<hr>"),
        BodyElement::Br(_) => html.push_str("<br>"),
        BodyElement::Script(s) => serialize_script(html, s),
    }
}

fn serialize_container(html: &mut String, tag: &str, c: &Container) {
    html.push('<');
    html.push_str(tag);
    if let Some(id) = &c.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &c.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');
    html.push_str(&escape_text(&c.text));
    for child in &c.children {
        serialize_element(html, child);
    }
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

fn serialize_text_element(html: &mut String, tag: &str, t: &TextElement) {
    html.push('<');
    html.push_str(tag);
    if let Some(id) = &t.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &t.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');
    html.push_str(&escape_text(&t.text));
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

fn serialize_list(html: &mut String, tag: &str, l: &ListContainer) {
    html.push('<');
    html.push_str(tag);
    if let Some(id) = &l.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &l.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');
    for item in &l.items {
        serialize_list_item(html, item);
    }
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

fn serialize_list_item(html: &mut String, item: &ListItem) {
    html.push_str("<li");
    if let Some(id) = &item.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &item.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');
    html.push_str(&escape_text(&item.text));
    for child in &item.children {
        serialize_element(html, child);
    }
    html.push_str("</li>");
}

fn serialize_anchor(html: &mut String, a: &Anchor) {
    html.push_str("<a");
    if let Some(id) = &a.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &a.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    if let Some(href) = &a.href {
        html.push_str(&format!(" href=\"{}\"", escape_attr(href)));
    }
    if let Some(target) = &a.target {
        html.push_str(&format!(" target=\"{}\"", escape_attr(target)));
    }
    html.push('>');
    html.push_str(&escape_text(&a.text));
    html.push_str("</a>");
}

fn serialize_image(html: &mut String, img: &Image) {
    html.push_str("<img");
    if let Some(id) = &img.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &img.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    if let Some(src) = &img.src {
        html.push_str(&format!(" src=\"{}\"", escape_attr(src)));
    }
    if let Some(alt) = &img.alt {
        html.push_str(&format!(" alt=\"{}\"", escape_attr(alt)));
    }
    if let Some(width) = &img.width {
        html.push_str(&format!(" width=\"{}\"", escape_attr(width)));
    }
    if let Some(height) = &img.height {
        html.push_str(&format!(" height=\"{}\"", escape_attr(height)));
    }
    html.push('>');
}

fn serialize_table(html: &mut String, t: &TableElement) {
    html.push_str("<table");
    if let Some(id) = &t.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &t.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    html.push('>');
    for child in &t.children {
        match child {
            TableChild::Thead(s) => serialize_table_section(html, "thead", s),
            TableChild::Tbody(s) => serialize_table_section(html, "tbody", s),
            TableChild::Tfoot(s) => serialize_table_section(html, "tfoot", s),
            TableChild::Tr(r) => serialize_table_row(html, r),
        }
    }
    html.push_str("</table>");
}

fn serialize_table_section(html: &mut String, tag: &str, s: &TableSection) {
    html.push('<');
    html.push_str(tag);
    html.push('>');
    for row in &s.rows {
        serialize_table_row(html, row);
    }
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

fn serialize_table_row(html: &mut String, r: &TableRow) {
    html.push_str("<tr>");
    for cell in &r.cells {
        match cell {
            TableCell::Th(t) => serialize_text_element(html, "th", t),
            TableCell::Td(t) => serialize_text_element(html, "td", t),
        }
    }
    html.push_str("</tr>");
}

fn serialize_input(html: &mut String, i: &InputElement) {
    html.push_str("<input");
    if let Some(id) = &i.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &i.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    if let Some(type_) = &i.type_ {
        html.push_str(&format!(" type=\"{}\"", escape_attr(type_)));
    }
    if let Some(name) = &i.name {
        html.push_str(&format!(" name=\"{}\"", escape_attr(name)));
    }
    if let Some(value) = &i.value {
        html.push_str(&format!(" value=\"{}\"", escape_attr(value)));
    }
    if let Some(placeholder) = &i.placeholder {
        html.push_str(&format!(" placeholder=\"{}\"", escape_attr(placeholder)));
    }
    html.push('>');
}

fn serialize_select(html: &mut String, s: &SelectElement) {
    html.push_str("<select");
    if let Some(id) = &s.id {
        html.push_str(&format!(" id=\"{}\"", escape_attr(id)));
    }
    if let Some(class) = &s.class {
        html.push_str(&format!(" class=\"{}\"", escape_attr(class)));
    }
    if let Some(name) = &s.name {
        html.push_str(&format!(" name=\"{}\"", escape_attr(name)));
    }
    html.push('>');
    for opt in &s.options {
        html.push_str("<option");
        if let Some(value) = &opt.value {
            html.push_str(&format!(" value=\"{}\"", escape_attr(value)));
        }
        if opt.selected.is_some() {
            html.push_str(" selected");
        }
        html.push('>');
        html.push_str(&escape_text(&opt.text));
        html.push_str("</option>");
    }
    html.push_str("</select>");
}

fn serialize_script(html: &mut String, s: &Script) {
    html.push_str("<script");
    if let Some(src) = &s.src {
        html.push_str(&format!(" src=\"{}\"", escape_attr(src)));
    }
    if let Some(type_) = &s.type_ {
        html.push_str(&format!(" type=\"{}\"", escape_attr(type_)));
    }
    if s.async_.is_some() {
        html.push_str(" async");
    }
    if s.defer.is_some() {
        html.push_str(" defer");
    }
    html.push('>');
    // Script content is not escaped
    html.push_str(&s.text);
    html.push_str("</script>");
}

/// Escape text content for HTML.
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape attribute value for HTML.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

rapace_cell::cell_service!(HtmlDifferServer<HtmlDifferImpl>, HtmlDifferImpl);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(HtmlDifferImpl)).await?;
    Ok(())
}
