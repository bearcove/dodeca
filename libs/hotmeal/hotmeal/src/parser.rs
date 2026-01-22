//! HTML5 parser using html5ever's TreeSink.
//!
//! This implements TreeSink to build hotmeal DOM types (Html, Body, Ul, Li, etc.)
//! using html5ever's tree construction algorithm, which includes browser-compatible error recovery.

use crate::diff::{Content, Element};
use crate::dom::*;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{Attribute, QualName, parse_document};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use tendril::StrTendril;

/// Parse an HTML string into a typed `Html` document.
///
/// Uses html5ever for browser-compatible parsing with full error recovery.
///
/// # Example
///
/// ```rust
/// use hotmeal::{parse, FlowContent};
///
/// let html = parse("<html><body><p>Hello!</p></body></html>");
/// if let Some(body) = &html.body {
///     if let Some(FlowContent::P(p)) = body.children.first() {
///         // Access the paragraph
///     }
/// }
/// ```
pub fn parse(html: &str) -> Html {
    let sink = HtmlSink::default();
    let sink = parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_html()
}

/// Parse an HTML string into an untyped Element tree (body content only).
///
/// This is more permissive than `parse()` - it accepts any HTML that browsers accept,
/// without enforcing HTML content model rules. Useful for diffing and patching.
///
/// Returns the body element with all its children.
pub fn parse_untyped(html: &str) -> Element {
    let sink = HtmlSink::default();
    let sink = parse_document(sink, Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();
    sink.into_untyped()
}

/// A node handle for our TreeSink - can be document, element, or text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeHandle(usize);

/// Internal node representation during parsing
#[derive(Clone, Debug)]
enum ParseNode {
    Document {
        children: Vec<NodeHandle>,
    },
    Element {
        name: QualName,
        attrs: Vec<(String, String)>,
        children: Vec<NodeHandle>,
    },
    Text(String),
}

/// TreeSink that builds hotmeal DOM types
#[derive(Default)]
struct HtmlSink {
    next_id: Cell<usize>,
    nodes: RefCell<HashMap<NodeHandle, ParseNode>>,
    document_handle: Cell<Option<NodeHandle>>,
}

impl HtmlSink {
    fn alloc(&self, node: ParseNode) -> NodeHandle {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let handle = NodeHandle(id);
        self.nodes.borrow_mut().insert(handle, node);
        handle
    }

    fn append_child_to(&self, parent: NodeHandle, child: NodeHandle) {
        let mut nodes = self.nodes.borrow_mut();
        match nodes.get_mut(&parent) {
            Some(ParseNode::Element { children, .. }) => children.push(child),
            Some(ParseNode::Document { children }) => children.push(child),
            _ => {}
        }
    }

    /// Convert the parsed tree to hotmeal Html type
    fn into_html(self) -> Html {
        let nodes = self.nodes.into_inner();
        let doc_handle = self.document_handle.get().unwrap();

        // Find html element
        if let Some(ParseNode::Document { children }) = nodes.get(&doc_handle) {
            for &child in children {
                if let Some(ParseNode::Element { name, .. }) = nodes.get(&child)
                    && name.local.as_ref() == "html"
                {
                    return Self::build_html(&nodes, child);
                }
            }
        }

        Html::default()
    }

    /// Convert the parsed tree to an untyped Element tree (body only).
    fn into_untyped(self) -> Element {
        let nodes = self.nodes.into_inner();
        let doc_handle = self.document_handle.get().unwrap();

        // Find html > body element
        if let Some(ParseNode::Document { children }) = nodes.get(&doc_handle) {
            for &child in children {
                if let Some(ParseNode::Element { name, children, .. }) = nodes.get(&child)
                    && name.local.as_ref() == "html"
                {
                    for &html_child in children {
                        if let Some(ParseNode::Element { name, .. }) = nodes.get(&html_child)
                            && name.local.as_ref() == "body"
                        {
                            return Self::build_untyped_element(&nodes, html_child);
                        }
                    }
                }
            }
        }

        // Fallback: empty body
        Element {
            tag: "body".to_string(),
            attrs: HashMap::new(),
            children: vec![],
        }
    }

    /// Recursively build an untyped Element from a ParseNode.
    fn build_untyped_element(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Element {
        if let Some(ParseNode::Element {
            name,
            attrs,
            children,
        }) = nodes.get(&handle)
        {
            Element {
                tag: name.local.to_string(),
                attrs: attrs.iter().cloned().collect(),
                children: children
                    .iter()
                    .filter_map(|&child| Self::build_untyped_content(nodes, child))
                    .collect(),
            }
        } else {
            Element {
                tag: "".to_string(),
                attrs: HashMap::new(),
                children: vec![],
            }
        }
    }

    /// Build untyped Content from a ParseNode.
    fn build_untyped_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<Content> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(Content::Text(t.clone())),
            ParseNode::Element { .. } => {
                Some(Content::Element(Self::build_untyped_element(nodes, handle)))
            }
            ParseNode::Document { .. } => None,
        }
    }

    fn build_html(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Html {
        let mut html = Html::default();

        if let Some(ParseNode::Element {
            children, attrs, ..
        }) = nodes.get(&handle)
        {
            // Set attributes
            for (name, value) in attrs {
                match name.as_str() {
                    "lang" => html.attrs.lang = Some(value.clone()),
                    "class" => html.attrs.class = Some(value.clone()),
                    "id" => html.attrs.id = Some(value.clone()),
                    _ => {
                        html.attrs.extra.insert(name.clone(), value.clone());
                    }
                }
            }

            // Find head and body
            for &child in children {
                if let Some(ParseNode::Element { name, .. }) = nodes.get(&child) {
                    match name.local.as_ref() {
                        "head" => html.head = Some(Self::build_head(nodes, child)),
                        "body" => html.body = Some(Self::build_body(nodes, child)),
                        _ => {}
                    }
                }
            }
        }

        html
    }

    fn build_head(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Head {
        let mut head = Head::default();

        if let Some(ParseNode::Element { children, .. }) = nodes.get(&handle) {
            for &child in children {
                if let Some(content) = Self::build_metadata_content(nodes, child) {
                    head.children.push(content);
                }
            }
        }

        head
    }

    fn build_metadata_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<MetadataContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(MetadataContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                match name.local.as_ref() {
                    "title" => {
                        let mut title = Title::default();
                        // Get text content
                        for &child in children {
                            if let Some(ParseNode::Text(t)) = nodes.get(&child) {
                                title.text.push_str(t);
                            }
                        }
                        Some(MetadataContent::Title(title))
                    }
                    "meta" => {
                        let mut meta = Meta::default();
                        for (k, v) in attrs {
                            match k.as_str() {
                                "name" => meta.name = Some(v.clone()),
                                "content" => meta.content = Some(v.clone()),
                                "charset" => meta.charset = Some(v.clone()),
                                _ => {}
                            }
                        }
                        Some(MetadataContent::Meta(meta))
                    }
                    _ => Some(MetadataContent::Text(String::new())),
                }
            }
            _ => None,
        }
    }

    fn build_body(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Body {
        let mut body = Body::default();

        if let Some(ParseNode::Element {
            children, attrs, ..
        }) = nodes.get(&handle)
        {
            // Set attributes
            for (name, value) in attrs {
                match name.as_str() {
                    "class" => body.attrs.class = Some(value.clone()),
                    "id" => body.attrs.id = Some(value.clone()),
                    _ => {
                        body.attrs.extra.insert(name.clone(), value.clone());
                    }
                }
            }

            // Build children
            for &child in children {
                if let Some(content) = Self::build_flow_content(nodes, child) {
                    body.children.push(content);
                }
            }
        }

        body
    }

    fn build_flow_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<FlowContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(FlowContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                let tag = name.local.as_ref();
                match tag {
                    "div" => {
                        let mut div = Div::default();
                        Self::set_global_attrs(&mut div.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_flow_content(nodes, child) {
                                div.children.push(c);
                            }
                        }
                        Some(FlowContent::Div(div))
                    }
                    "p" => {
                        let mut p = P::default();
                        Self::set_global_attrs(&mut p.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                p.children.push(c);
                            }
                        }
                        Some(FlowContent::P(p))
                    }
                    "ul" => {
                        let mut ul = Ul::default();
                        Self::set_global_attrs(&mut ul.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_ul_content(nodes, child) {
                                ul.children.push(c);
                            }
                        }
                        Some(FlowContent::Ul(ul))
                    }
                    "ol" => {
                        let mut ol = Ol::default();
                        Self::set_global_attrs(&mut ol.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_ol_content(nodes, child) {
                                ol.children.push(c);
                            }
                        }
                        Some(FlowContent::Ol(ol))
                    }
                    "table" => {
                        let mut table = Table::default();
                        Self::set_global_attrs(&mut table.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_table_content(nodes, child) {
                                table.children.push(c);
                            }
                        }
                        Some(FlowContent::Table(table))
                    }
                    "span" => {
                        let mut span = Span::default();
                        Self::set_global_attrs(&mut span.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                span.children.push(c);
                            }
                        }
                        Some(FlowContent::Span(span))
                    }
                    "a" => {
                        let mut a = A::default();
                        Self::set_global_attrs(&mut a.attrs, attrs);
                        for (k, v) in attrs {
                            if k == "href" {
                                a.href = Some(v.clone());
                            }
                        }
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                a.children.push(c);
                            }
                        }
                        Some(FlowContent::A(a))
                    }
                    "li" => {
                        // li at flow level - browser puts orphan li's here
                        // We'll wrap it in a custom element since Li isn't a FlowContent variant
                        let mut custom = CustomElement {
                            tag: "li".to_string(),
                            ..Default::default()
                        };
                        Self::set_global_attrs(&mut custom.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_flow_content(nodes, child) {
                                custom.children.push(c);
                            }
                        }
                        Some(FlowContent::Custom(custom))
                    }
                    _ => {
                        // Unknown element - use CustomElement
                        let mut custom = CustomElement {
                            tag: tag.to_string(),
                            ..Default::default()
                        };
                        Self::set_global_attrs(&mut custom.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_flow_content(nodes, child) {
                                custom.children.push(c);
                            }
                        }
                        Some(FlowContent::Custom(custom))
                    }
                }
            }
            _ => None,
        }
    }

    fn build_phrasing_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<PhrasingContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(PhrasingContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                let tag = name.local.as_ref();
                match tag {
                    "span" => {
                        let mut span = Span::default();
                        Self::set_global_attrs(&mut span.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                span.children.push(c);
                            }
                        }
                        Some(PhrasingContent::Span(span))
                    }
                    "strong" => {
                        let mut strong = Strong::default();
                        Self::set_global_attrs(&mut strong.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                strong.children.push(c);
                            }
                        }
                        Some(PhrasingContent::Strong(strong))
                    }
                    "em" => {
                        let mut em = Em::default();
                        Self::set_global_attrs(&mut em.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                em.children.push(c);
                            }
                        }
                        Some(PhrasingContent::Em(em))
                    }
                    "a" => {
                        let mut a = A::default();
                        Self::set_global_attrs(&mut a.attrs, attrs);
                        for (k, v) in attrs {
                            if k == "href" {
                                a.href = Some(v.clone());
                            }
                        }
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                a.children.push(c);
                            }
                        }
                        Some(PhrasingContent::A(a))
                    }
                    "code" => {
                        let mut code = Code::default();
                        Self::set_global_attrs(&mut code.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                code.children.push(c);
                            }
                        }
                        Some(PhrasingContent::Code(code))
                    }
                    _ => {
                        let mut custom = CustomPhrasingElement {
                            tag: tag.to_string(),
                            ..Default::default()
                        };
                        Self::set_global_attrs(&mut custom.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_phrasing_content(nodes, child) {
                                custom.children.push(c);
                            }
                        }
                        Some(PhrasingContent::Custom(custom))
                    }
                }
            }
            _ => None,
        }
    }

    fn build_ul_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<UlContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(UlContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                if name.local.as_ref() == "li" {
                    let mut li = Li::default();
                    Self::set_global_attrs(&mut li.attrs, attrs);
                    for (k, v) in attrs {
                        if k == "value" {
                            li.value = Some(v.clone());
                        }
                    }
                    for &child in children {
                        if let Some(c) = Self::build_flow_content(nodes, child) {
                            li.children.push(c);
                        }
                    }
                    Some(UlContent::Li(li))
                } else {
                    // Browser error recovery: non-li element inside ul
                    // We need to handle this - store as text placeholder for now
                    // In reality we'd need a Custom variant in UlContent
                    None
                }
            }
            _ => None,
        }
    }

    fn build_ol_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<OlContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(OlContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                if name.local.as_ref() == "li" {
                    let mut li = Li::default();
                    Self::set_global_attrs(&mut li.attrs, attrs);
                    for &child in children {
                        if let Some(c) = Self::build_flow_content(nodes, child) {
                            li.children.push(c);
                        }
                    }
                    Some(OlContent::Li(li))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn build_table_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<TableContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(TableContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                let tag = name.local.as_ref();
                match tag {
                    "thead" => {
                        let mut thead = Thead::default();
                        Self::set_global_attrs(&mut thead.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_table_section_content(nodes, child) {
                                thead.children.push(c);
                            }
                        }
                        Some(TableContent::Thead(thead))
                    }
                    "tbody" => {
                        let mut tbody = Tbody::default();
                        Self::set_global_attrs(&mut tbody.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_table_section_content(nodes, child) {
                                tbody.children.push(c);
                            }
                        }
                        Some(TableContent::Tbody(tbody))
                    }
                    "tfoot" => {
                        let mut tfoot = Tfoot::default();
                        Self::set_global_attrs(&mut tfoot.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_table_section_content(nodes, child) {
                                tfoot.children.push(c);
                            }
                        }
                        Some(TableContent::Tfoot(tfoot))
                    }
                    "tr" => {
                        let tr = Self::build_tr(nodes, handle)?;
                        Some(TableContent::Tr(tr))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn build_table_section_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<TableSectionContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(TableSectionContent::Text(t.clone())),
            ParseNode::Element { name, .. } => {
                if name.local.as_ref() == "tr" {
                    let tr = Self::build_tr(nodes, handle)?;
                    Some(TableSectionContent::Tr(tr))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn build_tr(nodes: &HashMap<NodeHandle, ParseNode>, handle: NodeHandle) -> Option<Tr> {
        if let Some(ParseNode::Element {
            attrs, children, ..
        }) = nodes.get(&handle)
        {
            let mut tr = Tr::default();
            Self::set_global_attrs(&mut tr.attrs, attrs);
            for &child in children {
                if let Some(c) = Self::build_tr_content(nodes, child) {
                    tr.children.push(c);
                }
            }
            Some(tr)
        } else {
            None
        }
    }

    fn build_tr_content(
        nodes: &HashMap<NodeHandle, ParseNode>,
        handle: NodeHandle,
    ) -> Option<TrContent> {
        match nodes.get(&handle)? {
            ParseNode::Text(t) => Some(TrContent::Text(t.clone())),
            ParseNode::Element {
                name,
                attrs,
                children,
            } => {
                let tag = name.local.as_ref();
                match tag {
                    "th" => {
                        let mut th = Th::default();
                        Self::set_global_attrs(&mut th.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_flow_content(nodes, child) {
                                th.children.push(c);
                            }
                        }
                        Some(TrContent::Th(th))
                    }
                    "td" => {
                        let mut td = Td::default();
                        Self::set_global_attrs(&mut td.attrs, attrs);
                        for &child in children {
                            if let Some(c) = Self::build_flow_content(nodes, child) {
                                td.children.push(c);
                            }
                        }
                        Some(TrContent::Td(td))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn set_global_attrs(attrs: &mut GlobalAttrs, src: &[(String, String)]) {
        for (k, v) in src {
            match k.as_str() {
                "id" => attrs.id = Some(v.clone()),
                "class" => attrs.class = Some(v.clone()),
                "style" => attrs.style = Some(v.clone()),
                "title" => attrs.tooltip = Some(v.clone()),
                _ => {
                    attrs.extra.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

impl TreeSink for HtmlSink {
    type Handle = NodeHandle;
    type Output = Self;
    type ElemName<'a> = &'a QualName;

    fn finish(self) -> Self::Output {
        self
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {
        // Ignore parse errors - we want to accept everything
    }

    fn get_document(&self) -> Self::Handle {
        if let Some(h) = self.document_handle.get() {
            h
        } else {
            let h = self.alloc(ParseNode::Document {
                children: Vec::new(),
            });
            self.document_handle.set(Some(h));
            h
        }
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        let nodes = self.nodes.borrow();
        if let Some(ParseNode::Element { name, .. }) = nodes.get(target) {
            unsafe { &*(name as *const QualName) }
        } else {
            panic!("elem_name called on non-element")
        }
    }

    fn create_element(
        &self,
        name: QualName,
        attrs: Vec<Attribute>,
        _flags: ElementFlags,
    ) -> Self::Handle {
        let attrs = attrs
            .into_iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect();
        self.alloc(ParseNode::Element {
            name,
            attrs,
            children: Vec::new(),
        })
    }

    fn create_comment(&self, _text: StrTendril) -> Self::Handle {
        // We don't care about comments
        self.alloc(ParseNode::Text(String::new()))
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Self::Handle {
        self.alloc(ParseNode::Text(String::new()))
    }

    fn append(&self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.append_child_to(*parent, node);
            }
            NodeOrText::AppendText(text) => {
                let mut nodes = self.nodes.borrow_mut();
                // Merge adjacent text nodes
                let last_child_id = match nodes.get(parent) {
                    Some(ParseNode::Element { children, .. }) => children.last().copied(),
                    Some(ParseNode::Document { children }) => children.last().copied(),
                    _ => None,
                };

                if let Some(last_id) = last_child_id
                    && let Some(ParseNode::Text(existing)) = nodes.get_mut(&last_id)
                {
                    existing.push_str(&text);
                    return;
                }
                drop(nodes);
                let text_id = self.alloc(ParseNode::Text(text.to_string()));
                self.append_child_to(*parent, text_id);
            }
        }
    }

    fn append_based_on_parent_node(
        &self,
        _element: &Self::Handle,
        prev_element: &Self::Handle,
        child: NodeOrText<Self::Handle>,
    ) {
        self.append(prev_element, child);
    }

    fn append_doctype_to_document(
        &self,
        _name: StrTendril,
        _public_id: StrTendril,
        _system_id: StrTendril,
    ) {
        // We don't store doctype
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        *target
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        x == y
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Self::Handle, new_node: NodeOrText<Self::Handle>) {
        let new_id = match new_node {
            NodeOrText::AppendNode(n) => n,
            NodeOrText::AppendText(text) => self.alloc(ParseNode::Text(text.to_string())),
        };

        let mut nodes = self.nodes.borrow_mut();
        for node in nodes.values_mut() {
            match node {
                ParseNode::Element { children, .. } | ParseNode::Document { children } => {
                    if let Some(pos) = children.iter().position(|&c| c == *sibling) {
                        children.insert(pos, new_id);
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(ParseNode::Element {
            attrs: existing, ..
        }) = nodes.get_mut(target)
        {
            for attr in attrs {
                let name = attr.name.local.to_string();
                if !existing.iter().any(|(k, _)| k == &name) {
                    existing.push((name, attr.value.to_string()));
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        let mut nodes = self.nodes.borrow_mut();
        for node in nodes.values_mut() {
            match node {
                ParseNode::Element { children, .. } | ParseNode::Document { children } => {
                    children.retain(|&c| c != *target);
                }
                _ => {}
            }
        }
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        let mut nodes = self.nodes.borrow_mut();
        let children = match nodes.get_mut(node) {
            Some(ParseNode::Element { children, .. }) => std::mem::take(children),
            Some(ParseNode::Document { children }) => std::mem::take(children),
            _ => return,
        };
        match nodes.get_mut(new_parent) {
            Some(ParseNode::Element {
                children: new_children,
                ..
            }) => new_children.extend(children),
            Some(ParseNode::Document {
                children: new_children,
            }) => new_children.extend(children),
            _ => {}
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ul() {
        let parsed = parse("<html><body><ul><li>Item A</li><li>Item B</li></ul></body></html>");

        let body = parsed.body.as_ref().expect("should have body");
        assert_eq!(body.children.len(), 1);

        if let FlowContent::Ul(ul) = &body.children[0] {
            assert_eq!(ul.children.len(), 2);
            if let UlContent::Li(li) = &ul.children[0] {
                assert_eq!(li.children.len(), 1);
                if let FlowContent::Text(t) = &li.children[0] {
                    assert_eq!(t, "Item A");
                } else {
                    panic!("expected text");
                }
            } else {
                panic!("expected li");
            }
        } else {
            panic!("expected ul, got {:?}", body.children[0]);
        }
    }

    #[test]
    fn test_ul_with_whitespace() {
        let parsed = parse(
            "<html><body><ul>\n    <li>Item A</li>\n    <li>Item B</li>\n</ul></body></html>",
        );

        let body = parsed.body.as_ref().expect("should have body");
        assert_eq!(body.children.len(), 1);

        if let FlowContent::Ul(ul) = &body.children[0] {
            // Should have: text, li, text, li, text (5 children)
            assert_eq!(
                ul.children.len(),
                5,
                "should preserve whitespace text nodes"
            );
        } else {
            panic!("expected ul");
        }
    }

    #[test]
    fn test_tr_outside_table() {
        // Browser strips table elements when outside table
        let parsed = parse("<html><body><tr><td>cell</td></tr></body></html>");

        let body = parsed.body.as_ref().expect("should have body");

        // Should just be text "cell" like browser does
        assert_eq!(body.children.len(), 1);
        if let FlowContent::Text(t) = &body.children[0] {
            assert_eq!(t, "cell");
        } else {
            panic!("expected text, got {:?}", body.children[0]);
        }
    }

    #[test]
    fn test_p_in_p() {
        // Browser auto-closes first p
        let parsed = parse("<html><body><p>outer<p>inner</p></p></body></html>");

        let body = parsed.body.as_ref().expect("should have body");

        // Browser creates: <p>outer</p><p>inner</p><p></p>
        // So we should have 3 children
        assert_eq!(body.children.len(), 3);
    }
}
