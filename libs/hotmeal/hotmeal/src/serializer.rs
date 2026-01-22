//! HTML serializer - turn typed DOM back into HTML strings.

use crate::*;
use std::fmt::Write;

/// Options for HTML serialization.
#[derive(Clone, Debug)]
pub struct SerializeOptions {
    /// Whether to pretty-print with indentation (default: false for minified output)
    pub pretty: bool,
    /// Indentation string for pretty-printing (default: "  ")
    pub indent: String,
}

impl Default for SerializeOptions {
    fn default() -> Self {
        Self {
            pretty: false,
            indent: "  ".to_string(),
        }
    }
}

impl SerializeOptions {
    /// Create new default options (minified output).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable pretty-printing with default indentation.
    pub fn pretty(mut self) -> Self {
        self.pretty = true;
        self
    }

    /// Set a custom indentation string (implies pretty-printing).
    pub fn indent(mut self, indent: impl Into<String>) -> Self {
        self.indent = indent.into();
        self.pretty = true;
        self
    }
}

/// Serialize an Html document to a string.
pub fn to_string(html: &Html) -> String {
    to_string_with_options(html, &SerializeOptions::default())
}

/// Serialize an Html document to a pretty-printed string.
pub fn to_string_pretty(html: &Html) -> String {
    to_string_with_options(html, &SerializeOptions::default().pretty())
}

/// Serialize an Html document to a string with custom options.
pub fn to_string_with_options(html: &Html, options: &SerializeOptions) -> String {
    let mut out = String::new();
    let mut ser = Serializer::new(&mut out, options);
    ser.write_html(html);
    out
}

struct Serializer<'a, W: Write> {
    out: &'a mut W,
    options: &'a SerializeOptions,
    depth: usize,
    in_raw_text: bool,
}

impl<'a, W: Write> Serializer<'a, W> {
    fn new(out: &'a mut W, options: &'a SerializeOptions) -> Self {
        Self {
            out,
            options,
            depth: 0,
            in_raw_text: false,
        }
    }

    fn write_indent(&mut self) {
        if self.options.pretty {
            for _ in 0..self.depth {
                let _ = write!(self.out, "{}", self.options.indent);
            }
        }
    }

    fn write_newline(&mut self) {
        if self.options.pretty {
            let _ = writeln!(self.out);
        }
    }

    fn write_text_escaped(&mut self, text: &str) {
        if self.in_raw_text {
            let _ = write!(self.out, "{}", text);
        } else {
            for c in text.chars() {
                match c {
                    '&' => {
                        let _ = write!(self.out, "&amp;");
                    }
                    '<' => {
                        let _ = write!(self.out, "&lt;");
                    }
                    '>' => {
                        let _ = write!(self.out, "&gt;");
                    }
                    _ => {
                        let _ = write!(self.out, "{}", c);
                    }
                }
            }
        }
    }

    fn write_attr_escaped(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '&' => {
                    let _ = write!(self.out, "&amp;");
                }
                '<' => {
                    let _ = write!(self.out, "&lt;");
                }
                '>' => {
                    let _ = write!(self.out, "&gt;");
                }
                '"' => {
                    let _ = write!(self.out, "&quot;");
                }
                _ => {
                    let _ = write!(self.out, "{}", c);
                }
            }
        }
    }

    fn write_attr(&mut self, name: &str, value: &str) {
        let _ = write!(self.out, " {}=\"", name);
        self.write_attr_escaped(value);
        let _ = write!(self.out, "\"");
    }

    fn write_global_attrs(&mut self, attrs: &GlobalAttrs) {
        if let Some(v) = &attrs.id {
            self.write_attr("id", v);
        }
        if let Some(v) = &attrs.class {
            self.write_attr("class", v);
        }
        if let Some(v) = &attrs.style {
            self.write_attr("style", v);
        }
        if let Some(v) = &attrs.tooltip {
            self.write_attr("title", v);
        }
        if let Some(v) = &attrs.lang {
            self.write_attr("lang", v);
        }
        if let Some(v) = &attrs.dir {
            self.write_attr("dir", v);
        }
        if let Some(v) = &attrs.hidden {
            self.write_attr("hidden", v);
        }
        if let Some(v) = &attrs.tabindex {
            self.write_attr("tabindex", v);
        }
        if let Some(v) = &attrs.accesskey {
            self.write_attr("accesskey", v);
        }
        if let Some(v) = &attrs.draggable {
            self.write_attr("draggable", v);
        }
        if let Some(v) = &attrs.contenteditable {
            self.write_attr("contenteditable", v);
        }
        if let Some(v) = &attrs.spellcheck {
            self.write_attr("spellcheck", v);
        }
        if let Some(v) = &attrs.translate {
            self.write_attr("translate", v);
        }
        if let Some(v) = &attrs.role {
            self.write_attr("role", v);
        }
        // Event handlers
        if let Some(v) = &attrs.onclick {
            self.write_attr("onclick", v);
        }
        if let Some(v) = &attrs.ondblclick {
            self.write_attr("ondblclick", v);
        }
        if let Some(v) = &attrs.onmousedown {
            self.write_attr("onmousedown", v);
        }
        if let Some(v) = &attrs.onmouseover {
            self.write_attr("onmouseover", v);
        }
        if let Some(v) = &attrs.onmouseout {
            self.write_attr("onmouseout", v);
        }
        if let Some(v) = &attrs.onmouseup {
            self.write_attr("onmouseup", v);
        }
        if let Some(v) = &attrs.onmouseenter {
            self.write_attr("onmouseenter", v);
        }
        if let Some(v) = &attrs.onmouseleave {
            self.write_attr("onmouseleave", v);
        }
        if let Some(v) = &attrs.onkeydown {
            self.write_attr("onkeydown", v);
        }
        if let Some(v) = &attrs.onkeyup {
            self.write_attr("onkeyup", v);
        }
        if let Some(v) = &attrs.onfocus {
            self.write_attr("onfocus", v);
        }
        if let Some(v) = &attrs.onblur {
            self.write_attr("onblur", v);
        }
        if let Some(v) = &attrs.onchange {
            self.write_attr("onchange", v);
        }
        if let Some(v) = &attrs.oninput {
            self.write_attr("oninput", v);
        }
        if let Some(v) = &attrs.onsubmit {
            self.write_attr("onsubmit", v);
        }
        if let Some(v) = &attrs.onload {
            self.write_attr("onload", v);
        }
        if let Some(v) = &attrs.onerror {
            self.write_attr("onerror", v);
        }
        if let Some(v) = &attrs.onscroll {
            self.write_attr("onscroll", v);
        }
        if let Some(v) = &attrs.oncontextmenu {
            self.write_attr("oncontextmenu", v);
        }
        // Extra attributes (data-*, aria-*, etc.)
        for (k, v) in &attrs.extra {
            self.write_attr(k, v);
        }
    }

    fn write_html(&mut self, html: &Html) {
        // DOCTYPE
        if let Some(doctype) = &html.doctype {
            let _ = write!(self.out, "<!DOCTYPE {}>", doctype);
            self.write_newline();
        }

        let _ = write!(self.out, "<html");
        self.write_global_attrs(&html.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();

        self.depth += 1;

        if let Some(head) = &html.head {
            self.write_head(head);
        }

        if let Some(body) = &html.body {
            self.write_body(body);
        }

        self.depth -= 1;

        let _ = write!(self.out, "</html>");
    }

    fn write_head(&mut self, head: &Head) {
        self.write_indent();
        let _ = write!(self.out, "<head");
        self.write_global_attrs(&head.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();

        self.depth += 1;
        for child in &head.children {
            self.write_metadata_content(child);
        }
        self.depth -= 1;

        self.write_indent();
        let _ = write!(self.out, "</head>");
        self.write_newline();
    }

    fn write_body(&mut self, body: &Body) {
        self.write_indent();
        let _ = write!(self.out, "<body");
        self.write_global_attrs(&body.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();

        self.depth += 1;
        for child in &body.children {
            self.write_flow_content(child);
        }
        self.depth -= 1;

        self.write_indent();
        let _ = write!(self.out, "</body>");
        self.write_newline();
    }

    fn write_metadata_content(&mut self, content: &MetadataContent) {
        match content {
            MetadataContent::Text(t) => {
                if !t.trim().is_empty() || !self.options.pretty {
                    self.write_text_escaped(t);
                }
            }
            MetadataContent::Title(title) => {
                self.write_indent();
                let _ = write!(self.out, "<title");
                self.write_global_attrs(&title.attrs);
                let _ = write!(self.out, ">");
                self.write_text_escaped(&title.text);
                let _ = write!(self.out, "</title>");
                self.write_newline();
            }
            MetadataContent::Base(base) => {
                self.write_indent();
                let _ = write!(self.out, "<base");
                self.write_global_attrs(&base.attrs);
                if let Some(v) = &base.href {
                    self.write_attr("href", v);
                }
                if let Some(v) = &base.target {
                    self.write_attr("target", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
            }
            MetadataContent::Link(link) => {
                self.write_indent();
                let _ = write!(self.out, "<link");
                self.write_global_attrs(&link.attrs);
                if let Some(v) = &link.href {
                    self.write_attr("href", v);
                }
                if let Some(v) = &link.rel {
                    self.write_attr("rel", v);
                }
                if let Some(v) = &link.type_ {
                    self.write_attr("type", v);
                }
                if let Some(v) = &link.media {
                    self.write_attr("media", v);
                }
                if let Some(v) = &link.integrity {
                    self.write_attr("integrity", v);
                }
                if let Some(v) = &link.crossorigin {
                    self.write_attr("crossorigin", v);
                }
                if let Some(v) = &link.sizes {
                    self.write_attr("sizes", v);
                }
                if let Some(v) = &link.as_ {
                    self.write_attr("as", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
            }
            MetadataContent::Meta(meta) => {
                self.write_indent();
                let _ = write!(self.out, "<meta");
                self.write_global_attrs(&meta.attrs);
                if let Some(v) = &meta.name {
                    self.write_attr("name", v);
                }
                if let Some(v) = &meta.content {
                    self.write_attr("content", v);
                }
                if let Some(v) = &meta.charset {
                    self.write_attr("charset", v);
                }
                if let Some(v) = &meta.http_equiv {
                    self.write_attr("http-equiv", v);
                }
                if let Some(v) = &meta.property {
                    self.write_attr("property", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
            }
            MetadataContent::Style(style) => {
                self.write_indent();
                let _ = write!(self.out, "<style");
                self.write_global_attrs(&style.attrs);
                if let Some(v) = &style.media {
                    self.write_attr("media", v);
                }
                if let Some(v) = &style.type_ {
                    self.write_attr("type", v);
                }
                let _ = write!(self.out, ">");
                // Style content is raw (not escaped)
                let _ = write!(self.out, "{}", style.text);
                let _ = write!(self.out, "</style>");
                self.write_newline();
            }
            MetadataContent::Script(script) => {
                self.write_script(script);
            }
            MetadataContent::Noscript(noscript) => {
                self.write_indent();
                let _ = write!(self.out, "<noscript");
                self.write_global_attrs(&noscript.attrs);
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &noscript.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</noscript>");
                self.write_newline();
            }
            MetadataContent::Template(template) => {
                self.write_indent();
                let _ = write!(self.out, "<template");
                self.write_global_attrs(&template.attrs);
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &template.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</template>");
                self.write_newline();
            }
        }
    }

    fn write_script(&mut self, script: &Script) {
        self.write_indent();
        let _ = write!(self.out, "<script");
        self.write_global_attrs(&script.attrs);
        if let Some(v) = &script.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &script.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &script.async_ {
            self.write_attr("async", v);
        }
        if let Some(v) = &script.defer {
            self.write_attr("defer", v);
        }
        if let Some(v) = &script.crossorigin {
            self.write_attr("crossorigin", v);
        }
        if let Some(v) = &script.integrity {
            self.write_attr("integrity", v);
        }
        if let Some(v) = &script.referrerpolicy {
            self.write_attr("referrerpolicy", v);
        }
        if let Some(v) = &script.nomodule {
            self.write_attr("nomodule", v);
        }
        let _ = write!(self.out, ">");
        // Script content is raw (not escaped)
        let _ = write!(self.out, "{}", script.text);
        let _ = write!(self.out, "</script>");
        self.write_newline();
    }

    fn write_flow_content(&mut self, content: &FlowContent) {
        match content {
            FlowContent::Text(t) => {
                if !t.trim().is_empty() || !self.options.pretty {
                    self.write_text_escaped(t);
                }
            }
            FlowContent::Header(el) => self.write_simple_block("header", &el.attrs, &el.children),
            FlowContent::Footer(el) => self.write_simple_block("footer", &el.attrs, &el.children),
            FlowContent::Main(el) => self.write_simple_block("main", &el.attrs, &el.children),
            FlowContent::Article(el) => self.write_simple_block("article", &el.attrs, &el.children),
            FlowContent::Section(el) => self.write_simple_block("section", &el.attrs, &el.children),
            FlowContent::Nav(el) => self.write_simple_block("nav", &el.attrs, &el.children),
            FlowContent::Aside(el) => self.write_simple_block("aside", &el.attrs, &el.children),
            FlowContent::Address(el) => self.write_simple_block("address", &el.attrs, &el.children),
            FlowContent::H1(el) => self.write_phrasing_block("h1", &el.attrs, &el.children),
            FlowContent::H2(el) => self.write_phrasing_block("h2", &el.attrs, &el.children),
            FlowContent::H3(el) => self.write_phrasing_block("h3", &el.attrs, &el.children),
            FlowContent::H4(el) => self.write_phrasing_block("h4", &el.attrs, &el.children),
            FlowContent::H5(el) => self.write_phrasing_block("h5", &el.attrs, &el.children),
            FlowContent::H6(el) => self.write_phrasing_block("h6", &el.attrs, &el.children),
            FlowContent::P(el) => self.write_phrasing_block("p", &el.attrs, &el.children),
            FlowContent::Div(el) => self.write_simple_block("div", &el.attrs, &el.children),
            FlowContent::Pre(el) => self.write_phrasing_block("pre", &el.attrs, &el.children),
            FlowContent::Blockquote(el) => {
                self.write_indent();
                let _ = write!(self.out, "<blockquote");
                self.write_global_attrs(&el.attrs);
                if let Some(v) = &el.cite {
                    self.write_attr("cite", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &el.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</blockquote>");
                self.write_newline();
            }
            FlowContent::Ol(ol) => self.write_ol(ol),
            FlowContent::Ul(ul) => self.write_ul(ul),
            FlowContent::Dl(dl) => self.write_dl(dl),
            FlowContent::Figure(fig) => {
                self.write_indent();
                let _ = write!(self.out, "<figure");
                self.write_global_attrs(&fig.attrs);
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                if let Some(caption) = &fig.figcaption {
                    self.write_indent();
                    let _ = write!(self.out, "<figcaption");
                    self.write_global_attrs(&caption.attrs);
                    let _ = write!(self.out, ">");
                    self.write_newline();
                    self.depth += 1;
                    for child in &caption.children {
                        self.write_flow_content(child);
                    }
                    self.depth -= 1;
                    self.write_indent();
                    let _ = write!(self.out, "</figcaption>");
                    self.write_newline();
                }
                for child in &fig.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</figure>");
                self.write_newline();
            }
            FlowContent::Hr(_hr) => {
                self.write_indent();
                let _ = write!(self.out, "<hr>");
                self.write_newline();
            }
            FlowContent::A(a) => self.write_a(a),
            FlowContent::Span(el) => self.write_phrasing_inline("span", &el.attrs, &el.children),
            FlowContent::Em(el) => self.write_phrasing_inline("em", &el.attrs, &el.children),
            FlowContent::Strong(el) => {
                self.write_phrasing_inline("strong", &el.attrs, &el.children)
            }
            FlowContent::Code(el) => self.write_phrasing_inline("code", &el.attrs, &el.children),
            FlowContent::Img(img) => self.write_img(img),
            FlowContent::Br(_) => {
                let _ = write!(self.out, "<br>");
            }
            FlowContent::Table(table) => self.write_table(table),
            FlowContent::Form(form) => self.write_form(form),
            FlowContent::Input(input) => self.write_input(input),
            FlowContent::Button(button) => self.write_button(button),
            FlowContent::Select(select) => self.write_select(select),
            FlowContent::Textarea(textarea) => self.write_textarea(textarea),
            FlowContent::Label(label) => self.write_label(label),
            FlowContent::Fieldset(fieldset) => self.write_fieldset(fieldset),
            FlowContent::Details(details) => self.write_details(details),
            FlowContent::Dialog(dialog) => {
                self.write_indent();
                let _ = write!(self.out, "<dialog");
                self.write_global_attrs(&dialog.attrs);
                if let Some(v) = &dialog.open {
                    self.write_attr("open", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &dialog.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</dialog>");
                self.write_newline();
            }
            FlowContent::Iframe(iframe) => self.write_iframe(iframe),
            FlowContent::Video(video) => self.write_video(video),
            FlowContent::Audio(audio) => self.write_audio(audio),
            FlowContent::Picture(picture) => self.write_picture(picture),
            FlowContent::Canvas(canvas) => {
                self.write_indent();
                let _ = write!(self.out, "<canvas");
                self.write_global_attrs(&canvas.attrs);
                if let Some(v) = &canvas.width {
                    self.write_attr("width", v);
                }
                if let Some(v) = &canvas.height {
                    self.write_attr("height", v);
                }
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &canvas.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</canvas>");
                self.write_newline();
            }
            FlowContent::Svg(svg) => self.write_svg(svg),
            FlowContent::Script(script) => self.write_script(script),
            FlowContent::Noscript(noscript) => {
                self.write_indent();
                let _ = write!(self.out, "<noscript");
                self.write_global_attrs(&noscript.attrs);
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &noscript.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</noscript>");
                self.write_newline();
            }
            FlowContent::Template(template) => {
                self.write_indent();
                let _ = write!(self.out, "<template");
                self.write_global_attrs(&template.attrs);
                let _ = write!(self.out, ">");
                self.write_newline();
                self.depth += 1;
                for child in &template.children {
                    self.write_flow_content(child);
                }
                self.depth -= 1;
                self.write_indent();
                let _ = write!(self.out, "</template>");
                self.write_newline();
            }
            FlowContent::Custom(custom) => {
                self.write_indent();
                let _ = write!(self.out, "<{}", custom.tag);
                self.write_global_attrs(&custom.attrs);
                let _ = write!(self.out, ">");
                if !custom.children.is_empty() {
                    self.write_newline();
                    self.depth += 1;
                    for child in &custom.children {
                        self.write_flow_content(child);
                    }
                    self.depth -= 1;
                    self.write_indent();
                }
                let _ = write!(self.out, "</{}>", custom.tag);
                self.write_newline();
            }
        }
    }

    fn write_simple_block(&mut self, tag: &str, attrs: &GlobalAttrs, children: &[FlowContent]) {
        self.write_indent();
        let _ = write!(self.out, "<{}", tag);
        self.write_global_attrs(attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in children {
            self.write_flow_content(child);
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</{}>", tag);
        self.write_newline();
    }

    fn write_phrasing_block(
        &mut self,
        tag: &str,
        attrs: &GlobalAttrs,
        children: &[PhrasingContent],
    ) {
        self.write_indent();
        let _ = write!(self.out, "<{}", tag);
        self.write_global_attrs(attrs);
        let _ = write!(self.out, ">");
        for child in children {
            self.write_phrasing_content(child);
        }
        let _ = write!(self.out, "</{}>", tag);
        self.write_newline();
    }

    fn write_phrasing_inline(
        &mut self,
        tag: &str,
        attrs: &GlobalAttrs,
        children: &[PhrasingContent],
    ) {
        let _ = write!(self.out, "<{}", tag);
        self.write_global_attrs(attrs);
        let _ = write!(self.out, ">");
        for child in children {
            self.write_phrasing_content(child);
        }
        let _ = write!(self.out, "</{}>", tag);
    }

    fn write_phrasing_content(&mut self, content: &PhrasingContent) {
        match content {
            PhrasingContent::Text(t) => self.write_text_escaped(t),
            PhrasingContent::A(a) => self.write_a(a),
            PhrasingContent::Span(el) => {
                self.write_phrasing_inline("span", &el.attrs, &el.children)
            }
            PhrasingContent::Em(el) => self.write_phrasing_inline("em", &el.attrs, &el.children),
            PhrasingContent::Strong(el) => {
                self.write_phrasing_inline("strong", &el.attrs, &el.children)
            }
            PhrasingContent::Small(el) => {
                self.write_phrasing_inline("small", &el.attrs, &el.children)
            }
            PhrasingContent::S(el) => self.write_phrasing_inline("s", &el.attrs, &el.children),
            PhrasingContent::Cite(el) => {
                self.write_phrasing_inline("cite", &el.attrs, &el.children)
            }
            PhrasingContent::Q(q) => {
                let _ = write!(self.out, "<q");
                self.write_global_attrs(&q.attrs);
                if let Some(v) = &q.cite {
                    self.write_attr("cite", v);
                }
                let _ = write!(self.out, ">");
                for child in &q.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</q>");
            }
            PhrasingContent::Dfn(el) => self.write_phrasing_inline("dfn", &el.attrs, &el.children),
            PhrasingContent::Abbr(el) => {
                self.write_phrasing_inline("abbr", &el.attrs, &el.children)
            }
            PhrasingContent::Data(data) => {
                let _ = write!(self.out, "<data");
                self.write_global_attrs(&data.attrs);
                if let Some(v) = &data.value {
                    self.write_attr("value", v);
                }
                let _ = write!(self.out, ">");
                for child in &data.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</data>");
            }
            PhrasingContent::Time(time) => {
                let _ = write!(self.out, "<time");
                self.write_global_attrs(&time.attrs);
                if let Some(v) = &time.datetime {
                    self.write_attr("datetime", v);
                }
                let _ = write!(self.out, ">");
                for child in &time.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</time>");
            }
            PhrasingContent::Code(el) => {
                self.write_phrasing_inline("code", &el.attrs, &el.children)
            }
            PhrasingContent::Var(el) => self.write_phrasing_inline("var", &el.attrs, &el.children),
            PhrasingContent::Samp(el) => {
                self.write_phrasing_inline("samp", &el.attrs, &el.children)
            }
            PhrasingContent::Kbd(el) => self.write_phrasing_inline("kbd", &el.attrs, &el.children),
            PhrasingContent::Sub(el) => self.write_phrasing_inline("sub", &el.attrs, &el.children),
            PhrasingContent::Sup(el) => self.write_phrasing_inline("sup", &el.attrs, &el.children),
            PhrasingContent::I(el) => self.write_phrasing_inline("i", &el.attrs, &el.children),
            PhrasingContent::B(el) => self.write_phrasing_inline("b", &el.attrs, &el.children),
            PhrasingContent::U(el) => self.write_phrasing_inline("u", &el.attrs, &el.children),
            PhrasingContent::Mark(el) => {
                self.write_phrasing_inline("mark", &el.attrs, &el.children)
            }
            PhrasingContent::Bdi(el) => self.write_phrasing_inline("bdi", &el.attrs, &el.children),
            PhrasingContent::Bdo(el) => self.write_phrasing_inline("bdo", &el.attrs, &el.children),
            PhrasingContent::Br(_) => {
                let _ = write!(self.out, "<br>");
            }
            PhrasingContent::Wbr(_) => {
                let _ = write!(self.out, "<wbr>");
            }
            PhrasingContent::Img(img) => self.write_img(img),
            PhrasingContent::Input(input) => self.write_input(input),
            PhrasingContent::Button(button) => self.write_button(button),
            PhrasingContent::Select(select) => self.write_select(select),
            PhrasingContent::Textarea(textarea) => self.write_textarea(textarea),
            PhrasingContent::Label(label) => self.write_label(label),
            PhrasingContent::Output(output) => {
                let _ = write!(self.out, "<output");
                self.write_global_attrs(&output.attrs);
                if let Some(v) = &output.for_ {
                    self.write_attr("for", v);
                }
                if let Some(v) = &output.name {
                    self.write_attr("name", v);
                }
                if let Some(v) = &output.form {
                    self.write_attr("form", v);
                }
                let _ = write!(self.out, ">");
                for child in &output.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</output>");
            }
            PhrasingContent::Progress(progress) => {
                let _ = write!(self.out, "<progress");
                self.write_global_attrs(&progress.attrs);
                if let Some(v) = &progress.value {
                    self.write_attr("value", v);
                }
                if let Some(v) = &progress.max {
                    self.write_attr("max", v);
                }
                let _ = write!(self.out, ">");
                for child in &progress.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</progress>");
            }
            PhrasingContent::Meter(meter) => {
                let _ = write!(self.out, "<meter");
                self.write_global_attrs(&meter.attrs);
                if let Some(v) = &meter.value {
                    self.write_attr("value", v);
                }
                if let Some(v) = &meter.min {
                    self.write_attr("min", v);
                }
                if let Some(v) = &meter.max {
                    self.write_attr("max", v);
                }
                if let Some(v) = &meter.low {
                    self.write_attr("low", v);
                }
                if let Some(v) = &meter.high {
                    self.write_attr("high", v);
                }
                if let Some(v) = &meter.optimum {
                    self.write_attr("optimum", v);
                }
                let _ = write!(self.out, ">");
                for child in &meter.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</meter>");
            }
            PhrasingContent::Script(script) => self.write_script(script),
            PhrasingContent::Custom(custom) => {
                let _ = write!(self.out, "<{}", custom.tag);
                self.write_global_attrs(&custom.attrs);
                let _ = write!(self.out, ">");
                for child in &custom.children {
                    self.write_phrasing_content(child);
                }
                let _ = write!(self.out, "</{}>", custom.tag);
            }
        }
    }

    fn write_a(&mut self, a: &A) {
        let _ = write!(self.out, "<a");
        self.write_global_attrs(&a.attrs);
        if let Some(v) = &a.href {
            self.write_attr("href", v);
        }
        if let Some(v) = &a.target {
            self.write_attr("target", v);
        }
        if let Some(v) = &a.rel {
            self.write_attr("rel", v);
        }
        if let Some(v) = &a.download {
            self.write_attr("download", v);
        }
        if let Some(v) = &a.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &a.hreflang {
            self.write_attr("hreflang", v);
        }
        if let Some(v) = &a.referrerpolicy {
            self.write_attr("referrerpolicy", v);
        }
        let _ = write!(self.out, ">");
        for child in &a.children {
            self.write_phrasing_content(child);
        }
        let _ = write!(self.out, "</a>");
    }

    fn write_img(&mut self, img: &Img) {
        let _ = write!(self.out, "<img");
        self.write_global_attrs(&img.attrs);
        if let Some(v) = &img.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &img.alt {
            self.write_attr("alt", v);
        }
        if let Some(v) = &img.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &img.height {
            self.write_attr("height", v);
        }
        if let Some(v) = &img.srcset {
            self.write_attr("srcset", v);
        }
        if let Some(v) = &img.sizes {
            self.write_attr("sizes", v);
        }
        if let Some(v) = &img.loading {
            self.write_attr("loading", v);
        }
        if let Some(v) = &img.decoding {
            self.write_attr("decoding", v);
        }
        if let Some(v) = &img.crossorigin {
            self.write_attr("crossorigin", v);
        }
        if let Some(v) = &img.referrerpolicy {
            self.write_attr("referrerpolicy", v);
        }
        if let Some(v) = &img.usemap {
            self.write_attr("usemap", v);
        }
        if let Some(v) = &img.ismap {
            self.write_attr("ismap", v);
        }
        let _ = write!(self.out, ">");
    }

    fn write_ol(&mut self, ol: &Ol) {
        self.write_indent();
        let _ = write!(self.out, "<ol");
        self.write_global_attrs(&ol.attrs);
        if let Some(v) = &ol.start {
            self.write_attr("start", v);
        }
        if let Some(v) = &ol.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &ol.reversed {
            self.write_attr("reversed", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &ol.children {
            match child {
                OlContent::Text(t) => {
                    if !t.trim().is_empty() || !self.options.pretty {
                        self.write_text_escaped(t);
                    }
                }
                OlContent::Li(li) => self.write_li(li),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</ol>");
        self.write_newline();
    }

    fn write_ul(&mut self, ul: &Ul) {
        self.write_indent();
        let _ = write!(self.out, "<ul");
        self.write_global_attrs(&ul.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &ul.children {
            match child {
                UlContent::Text(t) => {
                    if !t.trim().is_empty() || !self.options.pretty {
                        self.write_text_escaped(t);
                    }
                }
                UlContent::Li(li) => self.write_li(li),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</ul>");
        self.write_newline();
    }

    fn write_li(&mut self, li: &Li) {
        self.write_indent();
        let _ = write!(self.out, "<li");
        self.write_global_attrs(&li.attrs);
        if let Some(v) = &li.value {
            self.write_attr("value", v);
        }
        let _ = write!(self.out, ">");
        // Li can have mixed content
        let has_block = li
            .children
            .iter()
            .any(|c| !matches!(c, FlowContent::Text(_)));
        if has_block {
            self.write_newline();
            self.depth += 1;
            for child in &li.children {
                self.write_flow_content(child);
            }
            self.depth -= 1;
            self.write_indent();
        } else {
            for child in &li.children {
                self.write_flow_content(child);
            }
        }
        let _ = write!(self.out, "</li>");
        self.write_newline();
    }

    fn write_dl(&mut self, dl: &Dl) {
        self.write_indent();
        let _ = write!(self.out, "<dl");
        self.write_global_attrs(&dl.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &dl.children {
            match child {
                DlContent::Text(t) => {
                    if !t.trim().is_empty() || !self.options.pretty {
                        self.write_text_escaped(t);
                    }
                }
                DlContent::Dt(dt) => {
                    self.write_indent();
                    let _ = write!(self.out, "<dt");
                    self.write_global_attrs(&dt.attrs);
                    let _ = write!(self.out, ">");
                    for child in &dt.children {
                        self.write_flow_content(child);
                    }
                    let _ = write!(self.out, "</dt>");
                    self.write_newline();
                }
                DlContent::Dd(dd) => {
                    self.write_indent();
                    let _ = write!(self.out, "<dd");
                    self.write_global_attrs(&dd.attrs);
                    let _ = write!(self.out, ">");
                    for child in &dd.children {
                        self.write_flow_content(child);
                    }
                    let _ = write!(self.out, "</dd>");
                    self.write_newline();
                }
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</dl>");
        self.write_newline();
    }

    fn write_table(&mut self, table: &Table) {
        self.write_indent();
        let _ = write!(self.out, "<table");
        self.write_global_attrs(&table.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &table.children {
            match child {
                TableContent::Text(t) => {
                    if !t.trim().is_empty() || !self.options.pretty {
                        self.write_text_escaped(t);
                    }
                }
                TableContent::Caption(caption) => {
                    self.write_indent();
                    let _ = write!(self.out, "<caption");
                    self.write_global_attrs(&caption.attrs);
                    let _ = write!(self.out, ">");
                    for child in &caption.children {
                        self.write_flow_content(child);
                    }
                    let _ = write!(self.out, "</caption>");
                    self.write_newline();
                }
                TableContent::Colgroup(colgroup) => {
                    self.write_indent();
                    let _ = write!(self.out, "<colgroup");
                    self.write_global_attrs(&colgroup.attrs);
                    if let Some(v) = &colgroup.span {
                        self.write_attr("span", v);
                    }
                    let _ = write!(self.out, ">");
                    self.write_newline();
                    self.depth += 1;
                    for child in &colgroup.children {
                        match child {
                            ColgroupContent::Text(_) => {}
                            ColgroupContent::Col(col) => {
                                self.write_indent();
                                let _ = write!(self.out, "<col");
                                self.write_global_attrs(&col.attrs);
                                if let Some(v) = &col.span {
                                    self.write_attr("span", v);
                                }
                                let _ = write!(self.out, ">");
                                self.write_newline();
                            }
                        }
                    }
                    self.depth -= 1;
                    self.write_indent();
                    let _ = write!(self.out, "</colgroup>");
                    self.write_newline();
                }
                TableContent::Thead(thead) => {
                    self.write_table_section("thead", &thead.attrs, &thead.children)
                }
                TableContent::Tbody(tbody) => {
                    self.write_table_section("tbody", &tbody.attrs, &tbody.children)
                }
                TableContent::Tfoot(tfoot) => {
                    self.write_table_section("tfoot", &tfoot.attrs, &tfoot.children)
                }
                TableContent::Tr(tr) => self.write_tr(tr),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</table>");
        self.write_newline();
    }

    fn write_table_section(
        &mut self,
        tag: &str,
        attrs: &GlobalAttrs,
        children: &[TableSectionContent],
    ) {
        self.write_indent();
        let _ = write!(self.out, "<{}", tag);
        self.write_global_attrs(attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in children {
            match child {
                TableSectionContent::Text(_) => {}
                TableSectionContent::Tr(tr) => self.write_tr(tr),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</{}>", tag);
        self.write_newline();
    }

    fn write_tr(&mut self, tr: &Tr) {
        self.write_indent();
        let _ = write!(self.out, "<tr");
        self.write_global_attrs(&tr.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &tr.children {
            match child {
                TrContent::Text(_) => {}
                TrContent::Th(th) => {
                    self.write_indent();
                    let _ = write!(self.out, "<th");
                    self.write_global_attrs(&th.attrs);
                    if let Some(v) = &th.colspan {
                        self.write_attr("colspan", v);
                    }
                    if let Some(v) = &th.rowspan {
                        self.write_attr("rowspan", v);
                    }
                    if let Some(v) = &th.scope {
                        self.write_attr("scope", v);
                    }
                    if let Some(v) = &th.headers {
                        self.write_attr("headers", v);
                    }
                    if let Some(v) = &th.abbr {
                        self.write_attr("abbr", v);
                    }
                    let _ = write!(self.out, ">");
                    for child in &th.children {
                        self.write_flow_content(child);
                    }
                    let _ = write!(self.out, "</th>");
                    self.write_newline();
                }
                TrContent::Td(td) => {
                    self.write_indent();
                    let _ = write!(self.out, "<td");
                    self.write_global_attrs(&td.attrs);
                    if let Some(v) = &td.colspan {
                        self.write_attr("colspan", v);
                    }
                    if let Some(v) = &td.rowspan {
                        self.write_attr("rowspan", v);
                    }
                    if let Some(v) = &td.headers {
                        self.write_attr("headers", v);
                    }
                    let _ = write!(self.out, ">");
                    for child in &td.children {
                        self.write_flow_content(child);
                    }
                    let _ = write!(self.out, "</td>");
                    self.write_newline();
                }
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</tr>");
        self.write_newline();
    }

    fn write_form(&mut self, form: &Form) {
        self.write_indent();
        let _ = write!(self.out, "<form");
        self.write_global_attrs(&form.attrs);
        if let Some(v) = &form.action {
            self.write_attr("action", v);
        }
        if let Some(v) = &form.method {
            self.write_attr("method", v);
        }
        if let Some(v) = &form.enctype {
            self.write_attr("enctype", v);
        }
        if let Some(v) = &form.target {
            self.write_attr("target", v);
        }
        if let Some(v) = &form.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &form.autocomplete {
            self.write_attr("autocomplete", v);
        }
        if let Some(v) = &form.novalidate {
            self.write_attr("novalidate", v);
        }
        if let Some(v) = &form.accept_charset {
            self.write_attr("accept-charset", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &form.children {
            self.write_flow_content(child);
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</form>");
        self.write_newline();
    }

    fn write_input(&mut self, input: &Input) {
        let _ = write!(self.out, "<input");
        self.write_global_attrs(&input.attrs);
        if let Some(v) = &input.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &input.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &input.value {
            self.write_attr("value", v);
        }
        if let Some(v) = &input.placeholder {
            self.write_attr("placeholder", v);
        }
        if let Some(v) = &input.required {
            self.write_attr("required", v);
        }
        if let Some(v) = &input.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &input.readonly {
            self.write_attr("readonly", v);
        }
        if let Some(v) = &input.checked {
            self.write_attr("checked", v);
        }
        if let Some(v) = &input.autocomplete {
            self.write_attr("autocomplete", v);
        }
        if let Some(v) = &input.autofocus {
            self.write_attr("autofocus", v);
        }
        if let Some(v) = &input.min {
            self.write_attr("min", v);
        }
        if let Some(v) = &input.max {
            self.write_attr("max", v);
        }
        if let Some(v) = &input.step {
            self.write_attr("step", v);
        }
        if let Some(v) = &input.pattern {
            self.write_attr("pattern", v);
        }
        if let Some(v) = &input.size {
            self.write_attr("size", v);
        }
        if let Some(v) = &input.maxlength {
            self.write_attr("maxlength", v);
        }
        if let Some(v) = &input.minlength {
            self.write_attr("minlength", v);
        }
        if let Some(v) = &input.multiple {
            self.write_attr("multiple", v);
        }
        if let Some(v) = &input.accept {
            self.write_attr("accept", v);
        }
        if let Some(v) = &input.alt {
            self.write_attr("alt", v);
        }
        if let Some(v) = &input.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &input.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &input.height {
            self.write_attr("height", v);
        }
        if let Some(v) = &input.list {
            self.write_attr("list", v);
        }
        if let Some(v) = &input.form {
            self.write_attr("form", v);
        }
        if let Some(v) = &input.formaction {
            self.write_attr("formaction", v);
        }
        if let Some(v) = &input.formmethod {
            self.write_attr("formmethod", v);
        }
        if let Some(v) = &input.formenctype {
            self.write_attr("formenctype", v);
        }
        if let Some(v) = &input.formtarget {
            self.write_attr("formtarget", v);
        }
        if let Some(v) = &input.formnovalidate {
            self.write_attr("formnovalidate", v);
        }
        let _ = write!(self.out, ">");
    }

    fn write_button(&mut self, button: &Button) {
        let _ = write!(self.out, "<button");
        self.write_global_attrs(&button.attrs);
        if let Some(v) = &button.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &button.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &button.value {
            self.write_attr("value", v);
        }
        if let Some(v) = &button.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &button.autofocus {
            self.write_attr("autofocus", v);
        }
        if let Some(v) = &button.form {
            self.write_attr("form", v);
        }
        if let Some(v) = &button.formaction {
            self.write_attr("formaction", v);
        }
        if let Some(v) = &button.formmethod {
            self.write_attr("formmethod", v);
        }
        if let Some(v) = &button.formenctype {
            self.write_attr("formenctype", v);
        }
        if let Some(v) = &button.formtarget {
            self.write_attr("formtarget", v);
        }
        if let Some(v) = &button.formnovalidate {
            self.write_attr("formnovalidate", v);
        }
        let _ = write!(self.out, ">");
        for child in &button.children {
            self.write_phrasing_content(child);
        }
        let _ = write!(self.out, "</button>");
    }

    fn write_select(&mut self, select: &Select) {
        let _ = write!(self.out, "<select");
        self.write_global_attrs(&select.attrs);
        if let Some(v) = &select.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &select.multiple {
            self.write_attr("multiple", v);
        }
        if let Some(v) = &select.size {
            self.write_attr("size", v);
        }
        if let Some(v) = &select.required {
            self.write_attr("required", v);
        }
        if let Some(v) = &select.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &select.autofocus {
            self.write_attr("autofocus", v);
        }
        if let Some(v) = &select.autocomplete {
            self.write_attr("autocomplete", v);
        }
        if let Some(v) = &select.form {
            self.write_attr("form", v);
        }
        let _ = write!(self.out, ">");
        for child in &select.children {
            match child {
                SelectContent::Text(_) => {}
                SelectContent::Option(opt) => self.write_option(opt),
                SelectContent::Optgroup(optgroup) => {
                    let _ = write!(self.out, "<optgroup");
                    self.write_global_attrs(&optgroup.attrs);
                    if let Some(v) = &optgroup.label {
                        self.write_attr("label", v);
                    }
                    if let Some(v) = &optgroup.disabled {
                        self.write_attr("disabled", v);
                    }
                    let _ = write!(self.out, ">");
                    for child in &optgroup.children {
                        match child {
                            OptgroupContent::Text(_) => {}
                            OptgroupContent::Option(opt) => self.write_option(opt),
                        }
                    }
                    let _ = write!(self.out, "</optgroup>");
                }
            }
        }
        let _ = write!(self.out, "</select>");
    }

    fn write_option(&mut self, opt: &OptionElement) {
        let _ = write!(self.out, "<option");
        self.write_global_attrs(&opt.attrs);
        if let Some(v) = &opt.value {
            self.write_attr("value", v);
        }
        if let Some(v) = &opt.selected {
            self.write_attr("selected", v);
        }
        if let Some(v) = &opt.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &opt.label {
            self.write_attr("label", v);
        }
        let _ = write!(self.out, ">");
        self.write_text_escaped(&opt.text);
        let _ = write!(self.out, "</option>");
    }

    fn write_textarea(&mut self, textarea: &Textarea) {
        let _ = write!(self.out, "<textarea");
        self.write_global_attrs(&textarea.attrs);
        if let Some(v) = &textarea.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &textarea.rows {
            self.write_attr("rows", v);
        }
        if let Some(v) = &textarea.cols {
            self.write_attr("cols", v);
        }
        if let Some(v) = &textarea.placeholder {
            self.write_attr("placeholder", v);
        }
        if let Some(v) = &textarea.required {
            self.write_attr("required", v);
        }
        if let Some(v) = &textarea.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &textarea.readonly {
            self.write_attr("readonly", v);
        }
        if let Some(v) = &textarea.autofocus {
            self.write_attr("autofocus", v);
        }
        if let Some(v) = &textarea.autocomplete {
            self.write_attr("autocomplete", v);
        }
        if let Some(v) = &textarea.maxlength {
            self.write_attr("maxlength", v);
        }
        if let Some(v) = &textarea.minlength {
            self.write_attr("minlength", v);
        }
        if let Some(v) = &textarea.wrap {
            self.write_attr("wrap", v);
        }
        if let Some(v) = &textarea.form {
            self.write_attr("form", v);
        }
        let _ = write!(self.out, ">");
        self.write_text_escaped(&textarea.text);
        let _ = write!(self.out, "</textarea>");
    }

    fn write_label(&mut self, label: &Label) {
        let _ = write!(self.out, "<label");
        self.write_global_attrs(&label.attrs);
        if let Some(v) = &label.for_ {
            self.write_attr("for", v);
        }
        let _ = write!(self.out, ">");
        for child in &label.children {
            self.write_phrasing_content(child);
        }
        let _ = write!(self.out, "</label>");
    }

    fn write_fieldset(&mut self, fieldset: &Fieldset) {
        self.write_indent();
        let _ = write!(self.out, "<fieldset");
        self.write_global_attrs(&fieldset.attrs);
        if let Some(v) = &fieldset.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &fieldset.disabled {
            self.write_attr("disabled", v);
        }
        if let Some(v) = &fieldset.form {
            self.write_attr("form", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        if let Some(legend) = &fieldset.legend {
            self.write_indent();
            let _ = write!(self.out, "<legend");
            self.write_global_attrs(&legend.attrs);
            let _ = write!(self.out, ">");
            for child in &legend.children {
                self.write_phrasing_content(child);
            }
            let _ = write!(self.out, "</legend>");
            self.write_newline();
        }
        for child in &fieldset.children {
            self.write_flow_content(child);
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</fieldset>");
        self.write_newline();
    }

    fn write_details(&mut self, details: &Details) {
        self.write_indent();
        let _ = write!(self.out, "<details");
        self.write_global_attrs(&details.attrs);
        if let Some(v) = &details.open {
            self.write_attr("open", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        if let Some(summary) = &details.summary {
            self.write_indent();
            let _ = write!(self.out, "<summary");
            self.write_global_attrs(&summary.attrs);
            let _ = write!(self.out, ">");
            for child in &summary.children {
                self.write_phrasing_content(child);
            }
            let _ = write!(self.out, "</summary>");
            self.write_newline();
        }
        for child in &details.children {
            self.write_flow_content(child);
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</details>");
        self.write_newline();
    }

    fn write_iframe(&mut self, iframe: &Iframe) {
        self.write_indent();
        let _ = write!(self.out, "<iframe");
        self.write_global_attrs(&iframe.attrs);
        if let Some(v) = &iframe.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &iframe.srcdoc {
            self.write_attr("srcdoc", v);
        }
        if let Some(v) = &iframe.name {
            self.write_attr("name", v);
        }
        if let Some(v) = &iframe.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &iframe.height {
            self.write_attr("height", v);
        }
        if let Some(v) = &iframe.sandbox {
            self.write_attr("sandbox", v);
        }
        if let Some(v) = &iframe.allow {
            self.write_attr("allow", v);
        }
        if let Some(v) = &iframe.allowfullscreen {
            self.write_attr("allowfullscreen", v);
        }
        if let Some(v) = &iframe.loading {
            self.write_attr("loading", v);
        }
        if let Some(v) = &iframe.referrerpolicy {
            self.write_attr("referrerpolicy", v);
        }
        let _ = write!(self.out, "></iframe>");
        self.write_newline();
    }

    fn write_video(&mut self, video: &Video) {
        self.write_indent();
        let _ = write!(self.out, "<video");
        self.write_global_attrs(&video.attrs);
        if let Some(v) = &video.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &video.poster {
            self.write_attr("poster", v);
        }
        if let Some(v) = &video.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &video.height {
            self.write_attr("height", v);
        }
        if let Some(v) = &video.controls {
            self.write_attr("controls", v);
        }
        if let Some(v) = &video.autoplay {
            self.write_attr("autoplay", v);
        }
        if let Some(v) = &video.loop_ {
            self.write_attr("loop", v);
        }
        if let Some(v) = &video.muted {
            self.write_attr("muted", v);
        }
        if let Some(v) = &video.preload {
            self.write_attr("preload", v);
        }
        if let Some(v) = &video.playsinline {
            self.write_attr("playsinline", v);
        }
        if let Some(v) = &video.crossorigin {
            self.write_attr("crossorigin", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &video.children {
            match child {
                VideoContent::Text(_) => {}
                VideoContent::Source(source) => self.write_source(source),
                VideoContent::Track(track) => self.write_track(track),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</video>");
        self.write_newline();
    }

    fn write_audio(&mut self, audio: &Audio) {
        self.write_indent();
        let _ = write!(self.out, "<audio");
        self.write_global_attrs(&audio.attrs);
        if let Some(v) = &audio.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &audio.controls {
            self.write_attr("controls", v);
        }
        if let Some(v) = &audio.autoplay {
            self.write_attr("autoplay", v);
        }
        if let Some(v) = &audio.loop_ {
            self.write_attr("loop", v);
        }
        if let Some(v) = &audio.muted {
            self.write_attr("muted", v);
        }
        if let Some(v) = &audio.preload {
            self.write_attr("preload", v);
        }
        if let Some(v) = &audio.crossorigin {
            self.write_attr("crossorigin", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &audio.children {
            match child {
                AudioContent::Text(_) => {}
                AudioContent::Source(source) => self.write_source(source),
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</audio>");
        self.write_newline();
    }

    fn write_source(&mut self, source: &Source) {
        self.write_indent();
        let _ = write!(self.out, "<source");
        self.write_global_attrs(&source.attrs);
        if let Some(v) = &source.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &source.type_ {
            self.write_attr("type", v);
        }
        if let Some(v) = &source.srcset {
            self.write_attr("srcset", v);
        }
        if let Some(v) = &source.sizes {
            self.write_attr("sizes", v);
        }
        if let Some(v) = &source.media {
            self.write_attr("media", v);
        }
        if let Some(v) = &source.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &source.height {
            self.write_attr("height", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
    }

    fn write_track(&mut self, track: &Track) {
        self.write_indent();
        let _ = write!(self.out, "<track");
        self.write_global_attrs(&track.attrs);
        if let Some(v) = &track.src {
            self.write_attr("src", v);
        }
        if let Some(v) = &track.kind {
            self.write_attr("kind", v);
        }
        if let Some(v) = &track.srclang {
            self.write_attr("srclang", v);
        }
        if let Some(v) = &track.label {
            self.write_attr("label", v);
        }
        if let Some(v) = &track.default {
            self.write_attr("default", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
    }

    fn write_picture(&mut self, picture: &Picture) {
        self.write_indent();
        let _ = write!(self.out, "<picture");
        self.write_global_attrs(&picture.attrs);
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &picture.children {
            match child {
                PictureContent::Text(_) => {}
                PictureContent::Source(source) => self.write_source(source),
                PictureContent::Img(img) => {
                    self.write_indent();
                    self.write_img(img);
                    self.write_newline();
                }
            }
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</picture>");
        self.write_newline();
    }

    fn write_svg(&mut self, svg: &Svg) {
        self.write_indent();
        let _ = write!(self.out, "<svg");
        self.write_global_attrs(&svg.attrs);
        if let Some(v) = &svg.width {
            self.write_attr("width", v);
        }
        if let Some(v) = &svg.height {
            self.write_attr("height", v);
        }
        if let Some(v) = &svg.view_box {
            self.write_attr("viewBox", v);
        }
        if let Some(v) = &svg.xmlns {
            self.write_attr("xmlns", v);
        }
        if let Some(v) = &svg.preserve_aspect_ratio {
            self.write_attr("preserveAspectRatio", v);
        }
        let _ = write!(self.out, ">");
        self.write_newline();
        self.depth += 1;
        for child in &svg.children {
            self.write_svg_content(child);
        }
        self.depth -= 1;
        self.write_indent();
        let _ = write!(self.out, "</svg>");
        self.write_newline();
    }

    fn write_svg_content(&mut self, content: &SvgContent) {
        match content {
            SvgContent::TextNode(t) => {
                if !t.trim().is_empty() || !self.options.pretty {
                    self.write_text_escaped(t);
                }
            }
            SvgContent::Element(el) => {
                self.write_indent();
                let _ = write!(self.out, "<{}", el.tag);
                self.write_global_attrs(&el.attrs);
                let _ = write!(self.out, ">");
                if !el.children.is_empty() {
                    self.write_newline();
                    self.depth += 1;
                    for child in &el.children {
                        self.write_svg_content(child);
                    }
                    self.depth -= 1;
                    self.write_indent();
                }
                let _ = write!(self.out, "</{}>", el.tag);
                self.write_newline();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_roundtrip() {
        let html = "<html><body><p>Hello</p></body></html>";
        let doc = crate::parse(html);
        let output = to_string(&doc);
        assert!(output.contains("<p>Hello</p>"));
        assert!(output.contains("<body>"));
        assert!(output.contains("</body>"));
    }

    #[test]
    fn test_pretty_print() {
        let html = "<html><body><div><p>Hello</p></div></body></html>";
        let doc = crate::parse(html);
        let output = to_string_pretty(&doc);
        assert!(output.contains('\n'));
        assert!(output.contains("  ")); // indentation
    }

    #[test]
    fn test_attribute_escaping() {
        let html = r#"<html><body><a href="test&amp;foo">link</a></body></html>"#;
        let doc = crate::parse(html);
        let output = to_string(&doc);
        assert!(output.contains("href=\"test&amp;foo\""));
    }

    #[test]
    fn test_text_escaping() {
        let html = "<html><body><p>&lt;script&gt;</p></body></html>";
        let doc = crate::parse(html);
        let output = to_string(&doc);
        assert!(output.contains("&lt;script&gt;"));
    }
}
