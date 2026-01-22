//! Apply patches to HTML documents.
//!
//! For property testing: apply(A, diff(A, B)) == B

use super::translate::{InsertContent, NodePath, NodeRef, Patch, PropChange};
use crate::Html;
use std::collections::HashMap;
use std::fmt::Write;

/// A simple DOM element for patch application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Tag name
    pub tag: String,
    /// Attributes as key-value pairs
    pub attrs: HashMap<String, String>,
    /// Child nodes
    pub children: Vec<Content>,
}

/// DOM content - either an element or text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// An element node
    Element(Element),
    /// A text node
    Text(String),
}

impl Element {
    /// Get mutable reference to children at a path.
    pub fn children_mut(&mut self, path: &[usize]) -> Result<&mut Vec<Content>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => return Err("cannot navigate through text node".to_string()),
            };
        }
        Ok(&mut current.children)
    }

    /// Get mutable reference to attrs at a path.
    pub fn attrs_mut(&mut self, path: &[usize]) -> Result<&mut HashMap<String, String>, String> {
        let mut current = self;
        for &idx in path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => return Err("cannot navigate through text node".to_string()),
            };
        }
        Ok(&mut current.attrs)
    }

    /// Get mutable reference to content at a path.
    pub fn get_content_mut(&mut self, path: &[usize]) -> Result<&mut Content, String> {
        if path.is_empty() {
            return Err("cannot get content at empty path".to_string());
        }
        let parent_path = &path[..path.len() - 1];
        let idx = path[path.len() - 1];
        let children = self.children_mut(parent_path)?;
        children
            .get_mut(idx)
            .ok_or_else(|| format!("index {idx} out of bounds"))
    }

    /// Serialize this element to an HTML string (body content only).
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        self.write_html(&mut out);
        out
    }

    /// Write HTML to a string buffer.
    fn write_html(&self, out: &mut String) {
        write!(out, "<{}", self.tag).unwrap();

        // Sort attributes for deterministic output
        let mut attrs: Vec<_> = self.attrs.iter().collect();
        attrs.sort_by_key(|(k, _)| *k);
        for (name, value) in attrs {
            write!(out, " {}=\"{}\"", name, escape_attr(value)).unwrap();
        }
        out.push('>');

        for child in &self.children {
            child.write_html(out);
        }

        out.push_str("</");
        out.push_str(&self.tag);
        out.push('>');
    }

    /// Get concatenated text content of this element and all descendants.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for child in &self.children {
            match child {
                Content::Text(t) => out.push_str(t),
                Content::Element(e) => e.collect_text(out),
            }
        }
    }
}

impl Content {
    fn write_html(&self, out: &mut String) {
        match self {
            Content::Text(t) => out.push_str(&escape_text(t)),
            Content::Element(e) => e.write_html(out),
        }
    }
}

/// Escape text content for HTML.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape attribute value for HTML.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Parse an HTML string into an Element tree, returning the body.
pub fn parse_html(html: &str) -> Result<Element, String> {
    let doc: Html = crate::parse(html);
    let body_elem = typed_to_untyped_body(&doc);
    Ok(body_elem)
}

/// Convert the typed Html DOM to an untyped Element tree (body only).
fn typed_to_untyped_body(html: &Html) -> Element {
    use crate::*;

    fn convert_flow_content(content: &FlowContent) -> Content {
        match content {
            FlowContent::Text(t) => Content::Text(t.clone()),
            FlowContent::Div(div) => Content::Element(Element {
                tag: "div".to_string(),
                attrs: global_attrs_to_map(&div.attrs),
                children: div.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::P(p) => Content::Element(Element {
                tag: "p".to_string(),
                attrs: global_attrs_to_map(&p.attrs),
                children: p.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Span(span) => Content::Element(Element {
                tag: "span".to_string(),
                attrs: global_attrs_to_map(&span.attrs),
                children: span.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::A(a) => Content::Element(Element {
                tag: "a".to_string(),
                attrs: a_attrs_to_map(a),
                children: a.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Ul(ul) => Content::Element(Element {
                tag: "ul".to_string(),
                attrs: global_attrs_to_map(&ul.attrs),
                children: ul.children.iter().map(convert_ul_content).collect(),
            }),
            FlowContent::Ol(ol) => Content::Element(Element {
                tag: "ol".to_string(),
                attrs: global_attrs_to_map(&ol.attrs),
                children: ol.children.iter().map(convert_ol_content).collect(),
            }),
            FlowContent::Table(table) => Content::Element(Element {
                tag: "table".to_string(),
                attrs: global_attrs_to_map(&table.attrs),
                children: table.children.iter().map(convert_table_content).collect(),
            }),
            FlowContent::Strong(strong) => Content::Element(Element {
                tag: "strong".to_string(),
                attrs: global_attrs_to_map(&strong.attrs),
                children: strong
                    .children
                    .iter()
                    .map(convert_phrasing_content)
                    .collect(),
            }),
            FlowContent::Em(em) => Content::Element(Element {
                tag: "em".to_string(),
                attrs: global_attrs_to_map(&em.attrs),
                children: em.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Code(code) => Content::Element(Element {
                tag: "code".to_string(),
                attrs: global_attrs_to_map(&code.attrs),
                children: code.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Pre(pre) => Content::Element(Element {
                tag: "pre".to_string(),
                attrs: global_attrs_to_map(&pre.attrs),
                children: pre.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Blockquote(bq) => Content::Element(Element {
                tag: "blockquote".to_string(),
                attrs: global_attrs_to_map(&bq.attrs),
                children: bq.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::H1(h) => Content::Element(Element {
                tag: "h1".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::H2(h) => Content::Element(Element {
                tag: "h2".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::H3(h) => Content::Element(Element {
                tag: "h3".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::H4(h) => Content::Element(Element {
                tag: "h4".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::H5(h) => Content::Element(Element {
                tag: "h5".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::H6(h) => Content::Element(Element {
                tag: "h6".to_string(),
                attrs: global_attrs_to_map(&h.attrs),
                children: h.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Hr(_) => Content::Element(Element {
                tag: "hr".to_string(),
                attrs: HashMap::new(),
                children: vec![],
            }),
            FlowContent::Br(_) => Content::Element(Element {
                tag: "br".to_string(),
                attrs: HashMap::new(),
                children: vec![],
            }),
            FlowContent::Img(img) => Content::Element(Element {
                tag: "img".to_string(),
                attrs: img_attrs_to_map(img),
                children: vec![],
            }),
            FlowContent::Custom(custom) => Content::Element(Element {
                tag: custom.tag.clone(),
                attrs: global_attrs_to_map(&custom.attrs),
                children: custom.children.iter().map(convert_flow_content).collect(),
            }),
            // Section elements (all contain FlowContent children)
            FlowContent::Article(el) => Content::Element(Element {
                tag: "article".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Section(el) => Content::Element(Element {
                tag: "section".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Nav(el) => Content::Element(Element {
                tag: "nav".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Aside(el) => Content::Element(Element {
                tag: "aside".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Header(el) => Content::Element(Element {
                tag: "header".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Footer(el) => Content::Element(Element {
                tag: "footer".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Main(el) => Content::Element(Element {
                tag: "main".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Address(el) => Content::Element(Element {
                tag: "address".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            // Form elements
            FlowContent::Form(el) => Content::Element(Element {
                tag: "form".to_string(),
                attrs: form_attrs_to_map(el),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Fieldset(el) => {
                // Fieldset has optional legend + children: Vec<FlowContent>
                let mut children: Vec<Content> = Vec::new();
                if let Some(legend) = &el.legend {
                    children.push(Content::Element(Element {
                        tag: "legend".to_string(),
                        attrs: global_attrs_to_map(&legend.attrs),
                        children: legend
                            .children
                            .iter()
                            .map(convert_phrasing_content)
                            .collect(),
                    }));
                }
                children.extend(el.children.iter().map(convert_flow_content));
                Content::Element(Element {
                    tag: "fieldset".to_string(),
                    attrs: fieldset_attrs_to_map(el),
                    children,
                })
            }
            FlowContent::Label(el) => Content::Element(Element {
                tag: "label".to_string(),
                attrs: label_attrs_to_map(el),
                children: el.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Input(el) => Content::Element(Element {
                tag: "input".to_string(),
                attrs: input_attrs_to_map(el),
                children: vec![],
            }),
            FlowContent::Button(el) => Content::Element(Element {
                tag: "button".to_string(),
                attrs: button_attrs_to_map(el),
                children: el.children.iter().map(convert_phrasing_content).collect(),
            }),
            FlowContent::Select(el) => Content::Element(Element {
                tag: "select".to_string(),
                attrs: select_attrs_to_map(el),
                children: el.children.iter().map(convert_select_content).collect(),
            }),
            FlowContent::Textarea(el) => Content::Element(Element {
                tag: "textarea".to_string(),
                attrs: textarea_attrs_to_map(el),
                children: if el.text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(el.text.clone())]
                },
            }),
            // Other grouping elements
            FlowContent::Figure(el) => {
                // Figure has optional figcaption + children: Vec<FlowContent>
                let mut children: Vec<Content> =
                    el.children.iter().map(convert_flow_content).collect();
                if let Some(fc) = &el.figcaption {
                    children.push(Content::Element(Element {
                        tag: "figcaption".to_string(),
                        attrs: global_attrs_to_map(&fc.attrs),
                        children: fc.children.iter().map(convert_flow_content).collect(),
                    }));
                }
                Content::Element(Element {
                    tag: "figure".to_string(),
                    attrs: global_attrs_to_map(&el.attrs),
                    children,
                })
            }
            FlowContent::Dl(el) => Content::Element(Element {
                tag: "dl".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_dl_content).collect(),
            }),
            // Interactive elements
            FlowContent::Details(el) => {
                // Details has optional summary + children: Vec<FlowContent>
                let mut children: Vec<Content> = Vec::new();
                if let Some(summary) = &el.summary {
                    children.push(Content::Element(Element {
                        tag: "summary".to_string(),
                        attrs: global_attrs_to_map(&summary.attrs),
                        children: summary
                            .children
                            .iter()
                            .map(convert_phrasing_content)
                            .collect(),
                    }));
                }
                children.extend(el.children.iter().map(convert_flow_content));
                Content::Element(Element {
                    tag: "details".to_string(),
                    attrs: details_attrs_to_map(el),
                    children,
                })
            }
            FlowContent::Dialog(el) => Content::Element(Element {
                tag: "dialog".to_string(),
                attrs: dialog_attrs_to_map(el),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            // Embedded content
            FlowContent::Iframe(el) => Content::Element(Element {
                tag: "iframe".to_string(),
                attrs: iframe_attrs_to_map(el),
                children: vec![],
            }),
            FlowContent::Video(el) => Content::Element(Element {
                tag: "video".to_string(),
                attrs: video_attrs_to_map(el),
                children: el.children.iter().map(convert_video_content).collect(),
            }),
            FlowContent::Audio(el) => Content::Element(Element {
                tag: "audio".to_string(),
                attrs: audio_attrs_to_map(el),
                children: el.children.iter().map(convert_audio_content).collect(),
            }),
            FlowContent::Picture(el) => Content::Element(Element {
                tag: "picture".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_picture_content).collect(),
            }),
            FlowContent::Canvas(el) => Content::Element(Element {
                tag: "canvas".to_string(),
                attrs: canvas_attrs_to_map(el),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Svg(el) => Content::Element(Element {
                tag: "svg".to_string(),
                attrs: svg_attrs_to_map(el),
                children: vec![], // SVG content is complex, skip for now
            }),
            // Script/template elements
            FlowContent::Script(el) => Content::Element(Element {
                tag: "script".to_string(),
                attrs: script_attrs_to_map(el),
                children: if el.text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(el.text.clone())]
                },
            }),
            FlowContent::Noscript(el) => Content::Element(Element {
                tag: "noscript".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
            FlowContent::Template(el) => Content::Element(Element {
                tag: "template".to_string(),
                attrs: global_attrs_to_map(&el.attrs),
                children: el.children.iter().map(convert_flow_content).collect(),
            }),
        }
    }

    fn convert_phrasing_content(content: &PhrasingContent) -> Content {
        match content {
            PhrasingContent::Text(t) => Content::Text(t.clone()),
            PhrasingContent::Span(span) => Content::Element(Element {
                tag: "span".to_string(),
                attrs: global_attrs_to_map(&span.attrs),
                children: span.children.iter().map(convert_phrasing_content).collect(),
            }),
            PhrasingContent::Strong(strong) => Content::Element(Element {
                tag: "strong".to_string(),
                attrs: global_attrs_to_map(&strong.attrs),
                children: strong
                    .children
                    .iter()
                    .map(convert_phrasing_content)
                    .collect(),
            }),
            PhrasingContent::Em(em) => Content::Element(Element {
                tag: "em".to_string(),
                attrs: global_attrs_to_map(&em.attrs),
                children: em.children.iter().map(convert_phrasing_content).collect(),
            }),
            PhrasingContent::Code(code) => Content::Element(Element {
                tag: "code".to_string(),
                attrs: global_attrs_to_map(&code.attrs),
                children: code.children.iter().map(convert_phrasing_content).collect(),
            }),
            PhrasingContent::A(a) => Content::Element(Element {
                tag: "a".to_string(),
                attrs: a_attrs_to_map(a),
                children: a.children.iter().map(convert_phrasing_content).collect(),
            }),
            PhrasingContent::Br(_) => Content::Element(Element {
                tag: "br".to_string(),
                attrs: HashMap::new(),
                children: vec![],
            }),
            PhrasingContent::Img(img) => Content::Element(Element {
                tag: "img".to_string(),
                attrs: img_attrs_to_map(img),
                children: vec![],
            }),
            PhrasingContent::Custom(custom) => Content::Element(Element {
                tag: custom.tag.clone(),
                attrs: global_attrs_to_map(&custom.attrs),
                children: custom
                    .children
                    .iter()
                    .map(convert_phrasing_content)
                    .collect(),
            }),
            _ => Content::Text(String::new()),
        }
    }

    fn convert_ul_content(content: &UlContent) -> Content {
        match content {
            UlContent::Text(t) => Content::Text(t.clone()),
            UlContent::Li(li) => Content::Element(Element {
                tag: "li".to_string(),
                attrs: global_attrs_to_map(&li.attrs),
                children: li.children.iter().map(convert_flow_content).collect(),
            }),
        }
    }

    fn convert_ol_content(content: &OlContent) -> Content {
        match content {
            OlContent::Text(t) => Content::Text(t.clone()),
            OlContent::Li(li) => Content::Element(Element {
                tag: "li".to_string(),
                attrs: global_attrs_to_map(&li.attrs),
                children: li.children.iter().map(convert_flow_content).collect(),
            }),
        }
    }

    fn convert_table_content(content: &TableContent) -> Content {
        match content {
            TableContent::Text(t) => Content::Text(t.clone()),
            TableContent::Thead(thead) => Content::Element(Element {
                tag: "thead".to_string(),
                attrs: global_attrs_to_map(&thead.attrs),
                children: thead
                    .children
                    .iter()
                    .map(convert_table_section_content)
                    .collect(),
            }),
            TableContent::Tbody(tbody) => Content::Element(Element {
                tag: "tbody".to_string(),
                attrs: global_attrs_to_map(&tbody.attrs),
                children: tbody
                    .children
                    .iter()
                    .map(convert_table_section_content)
                    .collect(),
            }),
            TableContent::Tfoot(tfoot) => Content::Element(Element {
                tag: "tfoot".to_string(),
                attrs: global_attrs_to_map(&tfoot.attrs),
                children: tfoot
                    .children
                    .iter()
                    .map(convert_table_section_content)
                    .collect(),
            }),
            TableContent::Tr(tr) => Content::Element(Element {
                tag: "tr".to_string(),
                attrs: global_attrs_to_map(&tr.attrs),
                children: tr.children.iter().map(convert_tr_content).collect(),
            }),
            _ => Content::Text(String::new()),
        }
    }

    fn convert_table_section_content(content: &TableSectionContent) -> Content {
        match content {
            TableSectionContent::Text(t) => Content::Text(t.clone()),
            TableSectionContent::Tr(tr) => Content::Element(Element {
                tag: "tr".to_string(),
                attrs: global_attrs_to_map(&tr.attrs),
                children: tr.children.iter().map(convert_tr_content).collect(),
            }),
        }
    }

    fn convert_tr_content(content: &TrContent) -> Content {
        match content {
            TrContent::Text(t) => Content::Text(t.clone()),
            TrContent::Th(th) => Content::Element(Element {
                tag: "th".to_string(),
                attrs: global_attrs_to_map(&th.attrs),
                children: th.children.iter().map(convert_flow_content).collect(),
            }),
            TrContent::Td(td) => Content::Element(Element {
                tag: "td".to_string(),
                attrs: global_attrs_to_map(&td.attrs),
                children: td.children.iter().map(convert_flow_content).collect(),
            }),
        }
    }

    fn global_attrs_to_map(attrs: &GlobalAttrs) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(id) = &attrs.id {
            map.insert("id".to_string(), id.clone());
        }
        if let Some(class) = &attrs.class {
            map.insert("class".to_string(), class.clone());
        }
        if let Some(style) = &attrs.style {
            map.insert("style".to_string(), style.clone());
        }
        for (k, v) in &attrs.extra {
            map.insert(k.clone(), v.clone());
        }
        map
    }

    fn a_attrs_to_map(a: &A) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&a.attrs);
        if let Some(href) = &a.href {
            map.insert("href".to_string(), href.clone());
        }
        map
    }

    fn img_attrs_to_map(img: &Img) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&img.attrs);
        if let Some(src) = &img.src {
            map.insert("src".to_string(), src.clone());
        }
        if let Some(alt) = &img.alt {
            map.insert("alt".to_string(), alt.clone());
        }
        map
    }

    fn form_attrs_to_map(form: &Form) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&form.attrs);
        if let Some(action) = &form.action {
            map.insert("action".to_string(), action.clone());
        }
        if let Some(method) = &form.method {
            map.insert("method".to_string(), method.clone());
        }
        map
    }

    fn label_attrs_to_map(label: &Label) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&label.attrs);
        if let Some(for_attr) = &label.for_ {
            map.insert("for".to_string(), for_attr.clone());
        }
        map
    }

    fn input_attrs_to_map(input: &Input) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&input.attrs);
        if let Some(t) = &input.type_ {
            map.insert("type".to_string(), t.clone());
        }
        if let Some(name) = &input.name {
            map.insert("name".to_string(), name.clone());
        }
        if let Some(value) = &input.value {
            map.insert("value".to_string(), value.clone());
        }
        if let Some(placeholder) = &input.placeholder {
            map.insert("placeholder".to_string(), placeholder.clone());
        }
        map
    }

    fn button_attrs_to_map(button: &Button) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&button.attrs);
        if let Some(t) = &button.type_ {
            map.insert("type".to_string(), t.clone());
        }
        map
    }

    fn select_attrs_to_map(select: &Select) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&select.attrs);
        if let Some(name) = &select.name {
            map.insert("name".to_string(), name.clone());
        }
        map
    }

    fn textarea_attrs_to_map(textarea: &Textarea) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&textarea.attrs);
        if let Some(name) = &textarea.name {
            map.insert("name".to_string(), name.clone());
        }
        if let Some(placeholder) = &textarea.placeholder {
            map.insert("placeholder".to_string(), placeholder.clone());
        }
        map
    }

    fn details_attrs_to_map(details: &Details) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&details.attrs);
        if let Some(open) = &details.open {
            map.insert("open".to_string(), open.clone());
        }
        map
    }

    fn dialog_attrs_to_map(dialog: &Dialog) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&dialog.attrs);
        if let Some(open) = &dialog.open {
            map.insert("open".to_string(), open.clone());
        }
        map
    }

    fn iframe_attrs_to_map(iframe: &Iframe) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&iframe.attrs);
        if let Some(src) = &iframe.src {
            map.insert("src".to_string(), src.clone());
        }
        map
    }

    fn video_attrs_to_map(video: &Video) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&video.attrs);
        if let Some(src) = &video.src {
            map.insert("src".to_string(), src.clone());
        }
        map
    }

    fn audio_attrs_to_map(audio: &Audio) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&audio.attrs);
        if let Some(src) = &audio.src {
            map.insert("src".to_string(), src.clone());
        }
        map
    }

    fn canvas_attrs_to_map(canvas: &Canvas) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&canvas.attrs);
        if let Some(w) = &canvas.width {
            map.insert("width".to_string(), w.clone());
        }
        if let Some(h) = &canvas.height {
            map.insert("height".to_string(), h.clone());
        }
        map
    }

    fn svg_attrs_to_map(svg: &Svg) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&svg.attrs);
        if let Some(w) = &svg.width {
            map.insert("width".to_string(), w.clone());
        }
        if let Some(h) = &svg.height {
            map.insert("height".to_string(), h.clone());
        }
        if let Some(vb) = &svg.view_box {
            map.insert("viewBox".to_string(), vb.clone());
        }
        map
    }

    fn script_attrs_to_map(script: &Script) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&script.attrs);
        if let Some(src) = &script.src {
            map.insert("src".to_string(), src.clone());
        }
        map
    }

    fn fieldset_attrs_to_map(fieldset: &Fieldset) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&fieldset.attrs);
        if let Some(name) = &fieldset.name {
            map.insert("name".to_string(), name.clone());
        }
        if let Some(disabled) = &fieldset.disabled {
            map.insert("disabled".to_string(), disabled.clone());
        }
        if let Some(form) = &fieldset.form {
            map.insert("form".to_string(), form.clone());
        }
        map
    }

    fn convert_select_content(content: &SelectContent) -> Content {
        match content {
            SelectContent::Text(t) => Content::Text(t.clone()),
            SelectContent::Option(opt) => Content::Element(Element {
                tag: "option".to_string(),
                attrs: option_attrs_to_map(opt),
                children: if opt.text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(opt.text.clone())]
                },
            }),
            SelectContent::Optgroup(og) => Content::Element(Element {
                tag: "optgroup".to_string(),
                attrs: optgroup_attrs_to_map(og),
                children: og.children.iter().map(convert_optgroup_content).collect(),
            }),
        }
    }

    fn option_attrs_to_map(opt: &OptionElement) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&opt.attrs);
        if let Some(value) = &opt.value {
            map.insert("value".to_string(), value.clone());
        }
        if let Some(selected) = &opt.selected {
            map.insert("selected".to_string(), selected.clone());
        }
        map
    }

    fn optgroup_attrs_to_map(og: &Optgroup) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&og.attrs);
        if let Some(label) = &og.label {
            map.insert("label".to_string(), label.clone());
        }
        map
    }

    fn convert_optgroup_content(content: &OptgroupContent) -> Content {
        match content {
            OptgroupContent::Text(t) => Content::Text(t.clone()),
            OptgroupContent::Option(opt) => Content::Element(Element {
                tag: "option".to_string(),
                attrs: option_attrs_to_map(opt),
                children: if opt.text.is_empty() {
                    vec![]
                } else {
                    vec![Content::Text(opt.text.clone())]
                },
            }),
        }
    }

    fn convert_dl_content(content: &DlContent) -> Content {
        match content {
            DlContent::Text(t) => Content::Text(t.clone()),
            DlContent::Dt(dt) => Content::Element(Element {
                tag: "dt".to_string(),
                attrs: global_attrs_to_map(&dt.attrs),
                children: dt.children.iter().map(convert_flow_content).collect(),
            }),
            DlContent::Dd(dd) => Content::Element(Element {
                tag: "dd".to_string(),
                attrs: global_attrs_to_map(&dd.attrs),
                children: dd.children.iter().map(convert_flow_content).collect(),
            }),
        }
    }

    fn convert_video_content(content: &VideoContent) -> Content {
        match content {
            VideoContent::Text(t) => Content::Text(t.clone()),
            VideoContent::Source(source) => Content::Element(Element {
                tag: "source".to_string(),
                attrs: source_attrs_to_map(source),
                children: vec![],
            }),
            VideoContent::Track(track) => Content::Element(Element {
                tag: "track".to_string(),
                attrs: track_attrs_to_map(track),
                children: vec![],
            }),
        }
    }

    fn convert_audio_content(content: &AudioContent) -> Content {
        match content {
            AudioContent::Text(t) => Content::Text(t.clone()),
            AudioContent::Source(source) => Content::Element(Element {
                tag: "source".to_string(),
                attrs: source_attrs_to_map(source),
                children: vec![],
            }),
        }
    }

    fn source_attrs_to_map(source: &Source) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&source.attrs);
        if let Some(src) = &source.src {
            map.insert("src".to_string(), src.clone());
        }
        if let Some(t) = &source.type_ {
            map.insert("type".to_string(), t.clone());
        }
        if let Some(srcset) = &source.srcset {
            map.insert("srcset".to_string(), srcset.clone());
        }
        if let Some(media) = &source.media {
            map.insert("media".to_string(), media.clone());
        }
        map
    }

    fn track_attrs_to_map(track: &Track) -> HashMap<String, String> {
        let mut map = global_attrs_to_map(&track.attrs);
        if let Some(src) = &track.src {
            map.insert("src".to_string(), src.clone());
        }
        if let Some(kind) = &track.kind {
            map.insert("kind".to_string(), kind.clone());
        }
        map
    }

    fn convert_picture_content(content: &PictureContent) -> Content {
        match content {
            PictureContent::Text(t) => Content::Text(t.clone()),
            PictureContent::Source(source) => Content::Element(Element {
                tag: "source".to_string(),
                attrs: source_attrs_to_map(source),
                children: vec![],
            }),
            PictureContent::Img(img) => Content::Element(Element {
                tag: "img".to_string(),
                attrs: img_attrs_to_map(img),
                children: vec![],
            }),
        }
    }

    // Build body element
    let mut body_children = Vec::new();
    if let Some(body) = &html.body {
        for child in &body.children {
            body_children.push(convert_flow_content(child));
        }
    }

    Element {
        tag: "body".to_string(),
        attrs: HashMap::new(),
        children: body_children,
    }
}

/// Navigate within an element using a relative path (all but the last index) and return the children vec.
/// The last element of the path is the target index within the returned children.
/// Used for operations on nodes within detached slots.
fn navigate_to_children_in_slot<'a>(
    slot_node: &'a mut Element,
    rel_path: Option<&NodePath>,
) -> Result<&'a mut Vec<Content>, String> {
    let mut current = slot_node;
    if let Some(path) = rel_path {
        // Navigate all but the last segment (the last is the target index)
        let nav_path = if path.0.len() > 1 {
            &path.0[..path.0.len() - 1]
        } else {
            &[]
        };
        for &idx in nav_path {
            let child = current
                .children
                .get_mut(idx)
                .ok_or_else(|| format!("path index {idx} out of bounds in slot"))?;
            current = match child {
                Content::Element(e) => e,
                Content::Text(_) => {
                    return Err("cannot navigate through text node".to_string());
                }
            };
        }
    }
    Ok(&mut current.children)
}

/// Apply a list of patches to an Element tree in order.
pub fn apply_patches(root: &mut Element, patches: &[Patch]) -> Result<(), String> {
    // Slots hold Content (either Element or Text) that was displaced during edits
    let mut slots: HashMap<u32, Content> = HashMap::new();
    for patch in patches {
        apply_patch(root, patch, &mut slots)?;
    }
    Ok(())
}

/// Apply a single patch.
fn apply_patch(
    root: &mut Element,
    patch: &Patch,
    slots: &mut HashMap<u32, Content>,
) -> Result<(), String> {
    match patch {
        Patch::InsertElement {
            parent,
            position,
            tag,
            attrs,
            children,
            detach_to_slot,
        } => {
            // Create element with its attrs and children
            let new_element = Element {
                tag: tag.clone(),
                attrs: attrs.iter().cloned().collect(),
                children: children.iter().map(insert_content_to_content).collect(),
            };
            let new_content = Content::Element(new_element);

            insert_at_position(root, slots, parent, *position, new_content, *detach_to_slot)?;
        }
        Patch::InsertText {
            parent,
            position,
            text,
            detach_to_slot,
        } => {
            let new_content = Content::Text(text.clone());
            insert_at_position(root, slots, parent, *position, new_content, *detach_to_slot)?;
        }
        Patch::Remove { node } => {
            match node {
                NodeRef::Path(path) => {
                    if path.0.is_empty() {
                        return Err("Remove: cannot remove root".to_string());
                    }
                    let parent_path = &path.0[..path.0.len() - 1];
                    let idx = path.0[path.0.len() - 1];
                    let children = root
                        .children_mut(parent_path)
                        .map_err(|e| format!("Remove: {e}"))?;
                    if idx < children.len() {
                        // Swap with placeholder instead of remove (no shifting!)
                        children[idx] = Content::Text(String::new());
                    } else {
                        return Err(format!("Remove: index {idx} out of bounds"));
                    }
                }
                NodeRef::Slot(slot, _relative_path) => {
                    // Just remove from slots - the node was already detached
                    slots.remove(slot);
                }
            }
        }
        Patch::SetText { path, text } => {
            // Path points to a specific text node (e.g., [0, 1] = element at 0, text child at 1).
            // Navigate to the parent and replace just that child.
            if path.0.is_empty() {
                return Err("SetText: cannot set text on root".to_string());
            }
            let parent_path = &path.0[..path.0.len() - 1];
            let text_idx = path.0[path.0.len() - 1];
            let children = root
                .children_mut(parent_path)
                .map_err(|e| format!("SetText: {e}"))?;
            if text_idx >= children.len() {
                return Err(format!(
                    "SetText: index {text_idx} out of bounds (len={})",
                    children.len()
                ));
            }
            children[text_idx] = Content::Text(text.clone());
        }
        Patch::SetAttribute { path, name, value } => {
            let attrs = root
                .attrs_mut(&path.0)
                .map_err(|e| format!("SetAttribute: {e}"))?;
            attrs.insert(name.clone(), value.clone());
        }
        Patch::RemoveAttribute { path, name } => {
            let attrs = root
                .attrs_mut(&path.0)
                .map_err(|e| format!("RemoveAttribute: {e}"))?;
            attrs.remove(name);
        }
        Patch::Move {
            from,
            to,
            detach_to_slot,
        } => {
            debug!(?from, ?to, ?detach_to_slot, "apply Move");
            debug!(
                slots_before = ?slots.keys().collect::<Vec<_>>(),
                "apply Move slots state"
            );

            // Get the content to move (either from a path or from a slot)
            let content = match from {
                NodeRef::Path(from_path) => {
                    if from_path.0.is_empty() {
                        return Err("Move: cannot move root".to_string());
                    }
                    let from_parent_path = &from_path.0[..from_path.0.len() - 1];
                    let from_idx = from_path.0[from_path.0.len() - 1];
                    let from_children = root
                        .children_mut(from_parent_path)
                        .map_err(|e| format!("Move: {e}"))?;
                    if from_idx >= from_children.len() {
                        return Err(format!("Move: source index {from_idx} out of bounds"));
                    }
                    // Swap with placeholder instead of remove (no shifting!)
                    std::mem::replace(&mut from_children[from_idx], Content::Text(String::new()))
                }
                NodeRef::Slot(slot, _relative_path) => slots
                    .remove(slot)
                    .ok_or_else(|| format!("Move: slot {slot} not found"))?,
            };

            // Place the content at the target location (either in tree or in a slot)
            match to {
                NodeRef::Path(to_path) => {
                    if to_path.0.is_empty() {
                        return Err("Move: cannot move to root".to_string());
                    }
                    let to_parent_path = &to_path.0[..to_path.0.len() - 1];
                    let to_idx = to_path.0[to_path.0.len() - 1];

                    // Check if we need to detach the occupant at the target position
                    if let Some(slot) = detach_to_slot {
                        let to_children = root
                            .children_mut(to_parent_path)
                            .map_err(|e| format!("Move: {e}"))?;
                        debug!(
                            to_idx,
                            to_children_len = to_children.len(),
                            "Move detach check"
                        );
                        if to_idx < to_children.len() {
                            let occupant = std::mem::replace(
                                &mut to_children[to_idx],
                                Content::Text(String::new()),
                            );
                            debug!(slot, ?occupant, "Move detach: inserting occupant into slot");
                            slots.insert(*slot, occupant);
                        }
                    }

                    // Place the content at the target location
                    let to_children = root
                        .children_mut(to_parent_path)
                        .map_err(|e| format!("Move: {e}"))?;
                    // Grow the array with empty text placeholders if needed
                    while to_children.len() <= to_idx {
                        to_children.push(Content::Text(String::new()));
                    }
                    to_children[to_idx] = content;
                }
                NodeRef::Slot(target_slot, rel_path) => {
                    // Move into a slot (detached subtree)
                    // Get the target index from the relative path
                    let to_idx = rel_path
                        .as_ref()
                        .and_then(|p| p.0.last().copied())
                        .ok_or_else(|| "Move: slot target missing position index".to_string())?;

                    // Handle displacement if needed (in separate scope to release borrow)
                    if let Some(slot) = detach_to_slot {
                        let slot_content = slots
                            .get_mut(target_slot)
                            .ok_or_else(|| format!("Move: target slot {target_slot} not found"))?;

                        let target_children = match slot_content {
                            Content::Element(e) => {
                                navigate_to_children_in_slot(e, rel_path.as_ref())?
                            }
                            Content::Text(_) => {
                                return Err(
                                    "Move: target slot contains text, not element".to_string()
                                );
                            }
                        };

                        if to_idx < target_children.len() {
                            let occupant = std::mem::replace(
                                &mut target_children[to_idx],
                                Content::Text(String::new()),
                            );
                            slots.insert(*slot, occupant);
                        }
                    }

                    // Re-get the slot element (previous borrow was released)
                    let slot_content = slots
                        .get_mut(target_slot)
                        .ok_or_else(|| format!("Move: target slot {target_slot} not found"))?;

                    let target_children = match slot_content {
                        Content::Element(e) => navigate_to_children_in_slot(e, rel_path.as_ref())?,
                        Content::Text(_) => {
                            return Err("Move: target slot contains text, not element".to_string());
                        }
                    };

                    // Grow and place
                    while target_children.len() <= to_idx {
                        target_children.push(Content::Text(String::new()));
                    }
                    target_children[to_idx] = content;
                }
            }
        }
        Patch::UpdateProps { path, changes } => {
            apply_update_props(root, path, changes)?;
        }
    }
    Ok(())
}

/// Apply property updates, handling `_text` specially.
fn apply_update_props(
    root: &mut Element,
    path: &NodePath,
    changes: &[PropChange],
) -> Result<(), String> {
    // Get the content at path
    let content = root
        .get_content_mut(&path.0)
        .map_err(|e| format!("UpdateProps: {e}"))?;

    for change in changes {
        if change.name == "_text" {
            // Special handling for _text: update the text content directly
            match content {
                Content::Text(t) => {
                    if let Some(text) = &change.value {
                        *t = text.clone();
                    } else {
                        *t = String::new();
                    }
                }
                Content::Element(elem) => {
                    // Update text child of element
                    if let Some(text) = &change.value {
                        if elem.children.is_empty() {
                            elem.children.push(Content::Text(text.clone()));
                        } else {
                            let mut found_text = false;
                            for child in &mut elem.children {
                                if let Content::Text(t) = child {
                                    *t = text.clone();
                                    found_text = true;
                                    break;
                                }
                            }
                            if !found_text {
                                elem.children[0] = Content::Text(text.clone());
                            }
                        }
                    } else {
                        elem.children.retain(|c| !matches!(c, Content::Text(_)));
                    }
                }
            }
        } else {
            // Regular attribute - only valid on elements
            match content {
                Content::Element(elem) => {
                    if let Some(value) = &change.value {
                        elem.attrs.insert(change.name.clone(), value.clone());
                    } else {
                        elem.attrs.remove(&change.name);
                    }
                }
                Content::Text(_) => {
                    return Err(format!(
                        "UpdateProps: cannot set attribute '{}' on text node",
                        change.name
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Helper to insert content at a position, handling displacement to slots.
fn insert_at_position(
    root: &mut Element,
    slots: &mut HashMap<u32, Content>,
    parent: &NodeRef,
    position: usize,
    new_content: Content,
    detach_to_slot: Option<u32>,
) -> Result<(), String> {
    match parent {
        NodeRef::Path(path) => {
            let children = root
                .children_mut(&path.0)
                .map_err(|e| format!("Insert: {e}"))?;

            // In Chawathe semantics, Insert does NOT shift - it places at position
            // and whatever was there gets displaced (detached to a slot).
            if let Some(slot) = detach_to_slot
                && position < children.len()
            {
                let occupant =
                    std::mem::replace(&mut children[position], Content::Text(String::new()));
                slots.insert(slot, occupant);
            }

            // Grow the array with empty text placeholders if needed
            while children.len() <= position {
                children.push(Content::Text(String::new()));
            }
            children[position] = new_content;
        }
        NodeRef::Slot(parent_slot, relative_path) => {
            // Parent is in a slot - inserting into a detached subtree
            let slot_elem = match slots.get_mut(parent_slot) {
                Some(Content::Element(e)) => e,
                Some(Content::Text(_)) => {
                    return Err(format!(
                        "Insert: slot {parent_slot} contains text, not an element"
                    ));
                }
                None => return Err(format!("Insert: slot {parent_slot} not found")),
            };

            // First handle displacement if needed
            if let Some(slot) = detach_to_slot {
                let children = navigate_to_children_in_slot(slot_elem, relative_path.as_ref())?;
                if position < children.len() {
                    let occupant =
                        std::mem::replace(&mut children[position], Content::Text(String::new()));
                    slots.insert(slot, occupant);
                }
            }

            // Re-get the slot element (borrow was released)
            let slot_elem = match slots.get_mut(parent_slot) {
                Some(Content::Element(e)) => e,
                _ => return Err(format!("Insert: slot {parent_slot} not found")),
            };
            let children = navigate_to_children_in_slot(slot_elem, relative_path.as_ref())?;

            // Grow the array with empty text placeholders if needed
            while children.len() <= position {
                children.push(Content::Text(String::new()));
            }
            children[position] = new_content;
        }
    }
    Ok(())
}

/// Convert InsertContent to Content.
fn insert_content_to_content(ic: &InsertContent) -> Content {
    match ic {
        InsertContent::Element {
            tag,
            attrs,
            children,
        } => Content::Element(Element {
            tag: tag.clone(),
            attrs: attrs.iter().cloned().collect(),
            children: children.iter().map(insert_content_to_content).collect(),
        }),
        InsertContent::Text(s) => Content::Text(s.clone()),
    }
}
