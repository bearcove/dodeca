#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug, Clone)]
enum Node {
    Text(FuzzText),
    Comment(FuzzText),
    // Block elements
    Div {
        class: Option<AttrValue>,
        id: Option<AttrValue>,
        style: Option<AttrValue>,
        children: Vec<Node>,
    },
    P {
        class: Option<AttrValue>,
        text: FuzzText,
    },
    H1 {
        id: Option<AttrValue>,
        text: FuzzText,
    },
    H2 {
        text: FuzzText,
    },
    Section {
        class: Option<AttrValue>,
        children: Vec<Node>,
    },
    Article {
        children: Vec<Node>,
    },
    // Lists
    Ul {
        class: Option<AttrValue>,
        children: Vec<Node>,
    },
    Ol {
        start: Option<u8>,
        children: Vec<Node>,
    },
    Li {
        text: FuzzText,
    },
    // Table
    Table {
        children: Vec<Node>,
    },
    Tr {
        children: Vec<Node>,
    },
    Td {
        colspan: Option<u8>,
        children: Vec<Node>,
    },
    Th {
        text: FuzzText,
    },
    // Inline elements
    Span {
        class: Option<AttrValue>,
        text: FuzzText,
    },
    A {
        href: Option<AttrValue>,
        text: FuzzText,
    },
    Strong {
        text: FuzzText,
    },
    Em {
        text: FuzzText,
    },
    Code {
        text: FuzzText,
    },
    // Form elements
    Button {
        type_attr: Option<ButtonType>,
        text: FuzzText,
    },
    Input {
        type_attr: Option<InputType>,
        name: Option<AttrValue>,
        value: Option<AttrValue>,
    },
    Label {
        for_attr: Option<AttrValue>,
        text: FuzzText,
    },
    // Void elements
    Br,
    Hr,
    Img {
        src: Option<AttrValue>,
        alt: Option<AttrValue>,
    },
}

#[derive(Arbitrary, Debug, Clone)]
enum ButtonType {
    Submit,
    Button,
    Reset,
}

impl ButtonType {
    fn as_str(&self) -> &'static str {
        match self {
            ButtonType::Submit => "submit",
            ButtonType::Button => "button",
            ButtonType::Reset => "reset",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum InputType {
    Text,
    Email,
    Password,
    Checkbox,
    Radio,
    Number,
}

impl InputType {
    fn as_str(&self) -> &'static str {
        match self {
            InputType::Text => "text",
            InputType::Email => "email",
            InputType::Password => "password",
            InputType::Checkbox => "checkbox",
            InputType::Radio => "radio",
            InputType::Number => "number",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
struct FuzzText(
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| {
        let len = u.int_in_range(0..=30)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            // Include special chars that need escaping
            let c = match u.int_in_range::<u8>(0..=15)? {
                0..=10 => u.int_in_range(b'a'..=b'z')? as char,
                11 => '<',
                12 => '>',
                13 => '&',
                14 => '"',
                15 => ' ',
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(s)
    })]
    String,
);

#[derive(Arbitrary, Debug, Clone)]
struct AttrValue(
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| {
        let len = u.int_in_range(0..=20)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            // Attribute values - more restricted char set
            let c = match u.int_in_range::<u8>(0..=12)? {
                0..=8 => u.int_in_range(b'a'..=b'z')? as char,
                9 => '-',
                10 => '_',
                11 => '/',
                12 => '.',
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(s)
    })]
    String,
);

impl Node {
    fn to_html(&self, depth: usize) -> String {
        // Limit depth to prevent stack overflow
        if depth > 5 {
            return String::new();
        }

        // Helper to format children
        let fmt_children = |children: &[Node], limit: usize| -> String {
            children
                .iter()
                .take(limit)
                .map(|c| c.to_html(depth + 1))
                .collect()
        };

        match self {
            Node::Text(s) => s.0.clone(),
            Node::Comment(s) => format!("<!--{}-->", s.0),

            // Block elements
            Node::Div {
                class,
                id,
                style,
                children,
            } => {
                let mut attrs = String::new();
                if let Some(c) = class {
                    attrs.push_str(&format!(" class=\"{}\"", c.0));
                }
                if let Some(i) = id {
                    attrs.push_str(&format!(" id=\"{}\"", i.0));
                }
                if let Some(s) = style {
                    attrs.push_str(&format!(" style=\"{}\"", s.0));
                }
                format!("<div{}>{}</div>", attrs, fmt_children(children, 4))
            }
            Node::P { class, text } => {
                let class_attr = class
                    .as_ref()
                    .map(|c| format!(" class=\"{}\"", c.0))
                    .unwrap_or_default();
                format!("<p{}>{}</p>", class_attr, text.0)
            }
            Node::H1 { id, text } => {
                let id_attr = id
                    .as_ref()
                    .map(|i| format!(" id=\"{}\"", i.0))
                    .unwrap_or_default();
                format!("<h1{}>{}</h1>", id_attr, text.0)
            }
            Node::H2 { text } => format!("<h2>{}</h2>", text.0),
            Node::Section { class, children } => {
                let class_attr = class
                    .as_ref()
                    .map(|c| format!(" class=\"{}\"", c.0))
                    .unwrap_or_default();
                format!(
                    "<section{}>{}</section>",
                    class_attr,
                    fmt_children(children, 4)
                )
            }
            Node::Article { children } => {
                format!("<article>{}</article>", fmt_children(children, 4))
            }

            // Lists
            Node::Ul { class, children } => {
                let class_attr = class
                    .as_ref()
                    .map(|c| format!(" class=\"{}\"", c.0))
                    .unwrap_or_default();
                format!("<ul{}>{}</ul>", class_attr, fmt_children(children, 5))
            }
            Node::Ol { start, children } => {
                let start_attr = start
                    .as_ref()
                    .map(|s| format!(" start=\"{}\"", s))
                    .unwrap_or_default();
                format!("<ol{}>{}</ol>", start_attr, fmt_children(children, 5))
            }
            Node::Li { text } => format!("<li>{}</li>", text.0),

            // Tables
            Node::Table { children } => {
                format!("<table>{}</table>", fmt_children(children, 6))
            }
            Node::Tr { children } => {
                format!("<tr>{}</tr>", fmt_children(children, 6))
            }
            Node::Td { colspan, children } => {
                let colspan_attr = colspan
                    .as_ref()
                    .filter(|&&c| c > 1)
                    .map(|c| format!(" colspan=\"{}\"", c))
                    .unwrap_or_default();
                format!("<td{}>{}</td>", colspan_attr, fmt_children(children, 2))
            }
            Node::Th { text } => format!("<th>{}</th>", text.0),

            // Inline elements
            Node::Span { class, text } => {
                let class_attr = class
                    .as_ref()
                    .map(|c| format!(" class=\"{}\"", c.0))
                    .unwrap_or_default();
                format!("<span{}>{}</span>", class_attr, text.0)
            }
            Node::A { href, text } => {
                let href_attr = href
                    .as_ref()
                    .map(|h| format!(" href=\"{}\"", h.0))
                    .unwrap_or_default();
                format!("<a{}>{}</a>", href_attr, text.0)
            }
            Node::Strong { text } => format!("<strong>{}</strong>", text.0),
            Node::Em { text } => format!("<em>{}</em>", text.0),
            Node::Code { text } => format!("<code>{}</code>", text.0),

            // Form elements
            Node::Button { type_attr, text } => {
                let type_attr_str = type_attr
                    .as_ref()
                    .map(|t| format!(" type=\"{}\"", t.as_str()))
                    .unwrap_or_default();
                format!("<button{}>{}</button>", type_attr_str, text.0)
            }
            Node::Input {
                type_attr,
                name,
                value,
            } => {
                let mut attrs = String::new();
                if let Some(t) = type_attr {
                    attrs.push_str(&format!(" type=\"{}\"", t.as_str()));
                }
                if let Some(n) = name {
                    attrs.push_str(&format!(" name=\"{}\"", n.0));
                }
                if let Some(v) = value {
                    attrs.push_str(&format!(" value=\"{}\"", v.0));
                }
                format!("<input{}>", attrs)
            }
            Node::Label { for_attr, text } => {
                let for_attr_str = for_attr
                    .as_ref()
                    .map(|f| format!(" for=\"{}\"", f.0))
                    .unwrap_or_default();
                format!("<label{}>{}</label>", for_attr_str, text.0)
            }

            // Void elements
            Node::Br => "<br>".to_string(),
            Node::Hr => "<hr>".to_string(),
            Node::Img { src, alt } => {
                let mut attrs = String::new();
                if let Some(s) = src {
                    attrs.push_str(&format!(" src=\"{}\"", s.0));
                }
                if let Some(a) = alt {
                    attrs.push_str(&format!(" alt=\"{}\"", a.0));
                }
                format!("<img{}>", attrs)
            }
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum Doctype {
    None,
    Html5,
    Html4Strict,
    Html4Transitional,
    Xhtml1Strict,
}

impl Doctype {
    fn as_str(&self) -> &'static str {
        match self {
            Doctype::None => "",
            Doctype::Html5 => "<!DOCTYPE html>",
            Doctype::Html4Strict => {
                r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01//EN" "http://www.w3.org/TR/html4/strict.dtd">"#
            }
            Doctype::Html4Transitional => {
                r#"<!DOCTYPE HTML PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN" "http://www.w3.org/TR/html4/loose.dtd">"#
            }
            Doctype::Xhtml1Strict => {
                r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd">"#
            }
        }
    }
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    old_doctype: Doctype,
    old: Vec<Node>,
    new_doctype: Doctype,
    new: Vec<Node>,
    // Add some invalid nesting scenarios
    add_invalid_nesting: bool,
}

fn nodes_to_html(nodes: &[Node], doctype: &Doctype, add_invalid_nesting: bool) -> String {
    let mut inner: String = nodes.iter().take(5).map(|n| n.to_html(0)).collect();

    // Optionally add some invalid but browser-recoverable nesting
    if add_invalid_nesting && !inner.is_empty() {
        // Browsers auto-close p tags when they encounter block elements
        inner = format!(
            "<p>Unclosed paragraph{}<div>Inside P which browsers will auto-close</div>",
            inner
        );
        // Missing closing tags (browser adds them)
        inner.push_str("<span>Unclosed span<div>Block in span</div>");
    }

    format!("{}<html><body>{}</body></html>", doctype.as_str(), inner)
}

fuzz_target!(|input: FuzzInput| {
    let old_html = nodes_to_html(&input.old, &input.old_doctype, input.add_invalid_nesting);
    let new_html = nodes_to_html(&input.new, &input.new_doctype, false); // Don't add invalid nesting to new

    // Use arena_dom (the new implementation)
    let patches = hotmeal::diff::diff_html(&old_html, &new_html).expect("diff failed");
    let mut doc = hotmeal::arena_dom::parse(&old_html);
    // eprintln!("OLD HTML: {}", old_html);
    // eprintln!("NEW HTML: {}", new_html);
    // eprintln!("PATCHES: {:?}", patches);
    doc.apply_patches(&patches).expect("apply failed");

    let result = doc.to_html();
    let expected_doc = hotmeal::arena_dom::parse(&new_html);
    let expected = expected_doc.to_html();

    assert_eq!(
        result, expected,
        "Roundtrip failed!\nOld: {}\nNew: {}\nPatches: {:?}",
        old_html, new_html, patches
    );
});
