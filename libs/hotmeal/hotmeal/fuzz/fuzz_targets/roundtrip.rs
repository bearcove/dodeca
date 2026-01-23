#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

// =============================================================================
// CHAOS MODE: Truly arbitrary DOM generation
// =============================================================================

/// Arbitrary tag name - can be anything valid in HTML/XML
#[derive(Debug, Clone)]
struct ArbitraryTagName(String);

impl<'a> Arbitrary<'a> for ArbitraryTagName {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(1..=20)?;
        let mut s = String::with_capacity(len);

        // First char: letter or underscore (XML rules)
        let first = match u.int_in_range::<u8>(0..=52)? {
            0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
            26..=51 => (b'A' + u.int_in_range(0..=25u8)?) as char,
            52 => '_',
            _ => unreachable!(),
        };
        s.push(first);

        // Rest: letters, digits, hyphens, underscores, dots
        for _ in 1..len {
            let c = match u.int_in_range::<u8>(0..=65)? {
                0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
                26..=51 => (b'A' + u.int_in_range(0..=25u8)?) as char,
                52..=61 => (b'0' + u.int_in_range(0..=9u8)?) as char,
                62 => '-',
                63 => '_',
                64 => '.',
                65 => ':', // For namespaced tags like svg:rect
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(ArbitraryTagName(s))
    }
}

/// Arbitrary attribute name
#[derive(Debug, Clone)]
struct ArbitraryAttrName(String);

impl<'a> Arbitrary<'a> for ArbitraryAttrName {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Sometimes generate data-* attributes
        if u.ratio(1, 5)? {
            let suffix_len = u.int_in_range(1..=10)?;
            let mut s = String::from("data-");
            for _ in 0..suffix_len {
                let c = match u.int_in_range::<u8>(0..=27)? {
                    0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
                    26 => '-',
                    27 => '_',
                    _ => unreachable!(),
                };
                s.push(c);
            }
            return Ok(ArbitraryAttrName(s));
        }

        // Sometimes generate aria-* attributes
        if u.ratio(1, 5)? {
            let aria_attrs = [
                "aria-label",
                "aria-hidden",
                "aria-expanded",
                "aria-selected",
                "aria-disabled",
                "aria-describedby",
                "aria-labelledby",
                "aria-live",
                "aria-atomic",
                "aria-busy",
            ];
            return Ok(ArbitraryAttrName(u.choose(&aria_attrs)?.to_string()));
        }

        let len = u.int_in_range(1..=15)?;
        let mut s = String::with_capacity(len);

        // First char: letter
        let first = match u.int_in_range::<u8>(0..=51)? {
            0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
            26..=51 => (b'A' + u.int_in_range(0..=25u8)?) as char,
            _ => unreachable!(),
        };
        s.push(first);

        for _ in 1..len {
            let c = match u.int_in_range::<u8>(0..=65)? {
                0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
                26..=51 => (b'A' + u.int_in_range(0..=25u8)?) as char,
                52..=61 => (b'0' + u.int_in_range(0..=9u8)?) as char,
                62 => '-',
                63 => '_',
                64 => ':', // For namespaced attrs like xlink:href
                65 => '.',
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(ArbitraryAttrName(s))
    }
}

/// Arbitrary attribute value - can contain almost anything
#[derive(Debug, Clone)]
struct ArbitraryAttrValue(String);

impl<'a> Arbitrary<'a> for ArbitraryAttrValue {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(0..=50)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            let c = match u.int_in_range::<u8>(0..=40)? {
                0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
                26..=35 => (b'0' + u.int_in_range(0..=9u8)?) as char,
                36 => ' ',
                37 => '-',
                38 => '_',
                39 => '/',
                40 => '.',
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(ArbitraryAttrValue(s))
    }
}

/// Arbitrary text content - includes Unicode
#[derive(Debug, Clone)]
struct ArbitraryText(String);

impl<'a> Arbitrary<'a> for ArbitraryText {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(0..=100)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            let c = match u.int_in_range::<u8>(0..=60)? {
                // Letters
                0..=25 => (b'a' + u.int_in_range(0..=25u8)?) as char,
                26..=51 => (b'A' + u.int_in_range(0..=25u8)?) as char,
                // Digits
                52..=61 if u.int_in_range::<u8>(52..=61).is_ok() => {
                    (b'0' + u.int_in_range(0..=9u8)?) as char
                }
                // Whitespace
                52 => ' ',
                53 => '\t',
                54 => '\n',
                // Special HTML chars
                55 => '<',
                56 => '>',
                57 => '&',
                58 => '"',
                59 => '\'',
                // Unicode samples
                60 => {
                    let unicode_samples = [
                        'é', 'ñ', 'ü', 'ø', 'å', // Latin extended
                        '中', '文', '日', '本', // CJK
                        'α', 'β', 'γ', 'δ', 'π', // Greek
                        '→', '←', '↑', '↓', '•', // Symbols
                        '😀', '🎉', '🚀', '💯', // Emoji
                    ];
                    *u.choose(&unicode_samples)?
                }
                _ => ' ',
            };
            s.push(c);
        }
        Ok(ArbitraryText(s))
    }
}

/// A truly arbitrary attribute
#[derive(Arbitrary, Debug, Clone)]
struct ChaosAttr {
    name: ArbitraryAttrName,
    value: Option<ArbitraryAttrValue>, // None = boolean attribute
}

/// A truly arbitrary element
#[derive(Debug, Clone)]
struct ChaosElement {
    tag: ArbitraryTagName,
    attrs: Vec<ChaosAttr>,
    children: Vec<ChaosNode>,
    is_void: bool,
}

impl<'a> Arbitrary<'a> for ChaosElement {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let tag = ArbitraryTagName::arbitrary(u)?;
        let attr_count = u.int_in_range(0..=5)?;
        let mut attrs = Vec::with_capacity(attr_count);
        for _ in 0..attr_count {
            attrs.push(ChaosAttr::arbitrary(u)?);
        }

        // HTML void elements
        let void_tags = [
            "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
            "source", "track", "wbr",
        ];
        let is_void = void_tags.contains(&tag.0.to_lowercase().as_str()) || u.ratio(1, 20)?;

        let children = if is_void {
            vec![]
        } else {
            let child_count = u.int_in_range(0..=4)?;
            let mut children = Vec::with_capacity(child_count);
            for _ in 0..child_count {
                children.push(ChaosNode::arbitrary(u)?);
            }
            children
        };

        Ok(ChaosElement {
            tag,
            attrs,
            children,
            is_void,
        })
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum ChaosNode {
    Text(ArbitraryText),
    Comment(ArbitraryText),
    Element(Box<ChaosElement>),
}

impl ChaosNode {
    fn to_html(&self, depth: usize) -> String {
        if depth > 6 {
            return String::new();
        }

        match self {
            ChaosNode::Text(t) => html_escape_text(&t.0),
            ChaosNode::Comment(t) => {
                // Comments can't contain -- or end with -
                let safe = t.0.replace("--", "- -").trim_end_matches('-').to_string();
                format!("<!--{}-->", safe)
            }
            ChaosNode::Element(el) => {
                let mut html = String::new();
                html.push('<');
                html.push_str(&el.tag.0);

                for attr in &el.attrs {
                    html.push(' ');
                    html.push_str(&attr.name.0);
                    if let Some(val) = &attr.value {
                        html.push_str("=\"");
                        html.push_str(&html_escape_attr(&val.0));
                        html.push('"');
                    }
                }

                if el.is_void {
                    html.push_str(">");
                } else {
                    html.push('>');
                    for child in &el.children {
                        html.push_str(&child.to_html(depth + 1));
                    }
                    html.push_str("</");
                    html.push_str(&el.tag.0);
                    html.push('>');
                }
                html
            }
        }
    }
}

// =============================================================================
// EXTENDED REALISTIC MODE: More element types, but structured
// =============================================================================

#[derive(Arbitrary, Debug, Clone)]
enum ExtendedNode {
    // Original nodes
    Text(FuzzText),
    Comment(FuzzText),

    // Block elements
    Div {
        class: Option<AttrValue>,
        id: Option<AttrValue>,
        style: Option<AttrValue>,
        data_attrs: Vec<DataAttr>,
        children: Vec<ExtendedNode>,
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
    H3 {
        text: FuzzText,
    },
    H4 {
        text: FuzzText,
    },
    H5 {
        text: FuzzText,
    },
    H6 {
        text: FuzzText,
    },
    Section {
        class: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Article {
        children: Vec<ExtendedNode>,
    },
    Aside {
        children: Vec<ExtendedNode>,
    },
    Header {
        children: Vec<ExtendedNode>,
    },
    Footer {
        children: Vec<ExtendedNode>,
    },
    Nav {
        children: Vec<ExtendedNode>,
    },
    Main {
        children: Vec<ExtendedNode>,
    },

    // Lists
    Ul {
        class: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Ol {
        start: Option<u8>,
        children: Vec<ExtendedNode>,
    },
    Li {
        class: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Dl {
        children: Vec<ExtendedNode>,
    },
    Dt {
        text: FuzzText,
    },
    Dd {
        text: FuzzText,
    },

    // Table
    Table {
        class: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Thead {
        children: Vec<ExtendedNode>,
    },
    Tbody {
        children: Vec<ExtendedNode>,
    },
    Tfoot {
        children: Vec<ExtendedNode>,
    },
    Tr {
        children: Vec<ExtendedNode>,
    },
    Td {
        colspan: Option<u8>,
        rowspan: Option<u8>,
        children: Vec<ExtendedNode>,
    },
    Th {
        scope: Option<ThScope>,
        text: FuzzText,
    },
    Caption {
        text: FuzzText,
    },

    // Inline elements
    Span {
        class: Option<AttrValue>,
        text: FuzzText,
    },
    A {
        href: Option<AttrValue>,
        target: Option<ATarget>,
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
    Pre {
        children: Vec<ExtendedNode>,
    },
    Blockquote {
        cite: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Q {
        text: FuzzText,
    },
    Abbr {
        title: Option<AttrValue>,
        text: FuzzText,
    },
    Time {
        datetime: Option<AttrValue>,
        text: FuzzText,
    },
    Mark {
        text: FuzzText,
    },
    Small {
        text: FuzzText,
    },
    Sub {
        text: FuzzText,
    },
    Sup {
        text: FuzzText,
    },
    Kbd {
        text: FuzzText,
    },
    Samp {
        text: FuzzText,
    },
    Var {
        text: FuzzText,
    },
    Cite {
        text: FuzzText,
    },
    Dfn {
        text: FuzzText,
    },
    Del {
        text: FuzzText,
    },
    Ins {
        text: FuzzText,
    },
    S {
        text: FuzzText,
    },
    U {
        text: FuzzText,
    },
    B {
        text: FuzzText,
    },
    I {
        text: FuzzText,
    },

    // Form elements
    Form {
        action: Option<AttrValue>,
        method: Option<FormMethod>,
        children: Vec<ExtendedNode>,
    },
    Button {
        type_attr: Option<ButtonType>,
        disabled: bool,
        text: FuzzText,
    },
    Input {
        type_attr: Option<InputType>,
        name: Option<AttrValue>,
        value: Option<AttrValue>,
        placeholder: Option<AttrValue>,
        disabled: bool,
        readonly: bool,
        required: bool,
    },
    Label {
        for_attr: Option<AttrValue>,
        text: FuzzText,
    },
    Select {
        name: Option<AttrValue>,
        multiple: bool,
        disabled: bool,
        children: Vec<ExtendedNode>,
    },
    Option {
        value: Option<AttrValue>,
        selected: bool,
        disabled: bool,
        text: FuzzText,
    },
    Optgroup {
        label: AttrValue,
        disabled: bool,
        children: Vec<ExtendedNode>,
    },
    Textarea {
        name: Option<AttrValue>,
        rows: Option<u8>,
        cols: Option<u8>,
        placeholder: Option<AttrValue>,
        disabled: bool,
        readonly: bool,
        text: FuzzText,
    },
    Fieldset {
        disabled: bool,
        children: Vec<ExtendedNode>,
    },
    Legend {
        text: FuzzText,
    },
    Datalist {
        id: AttrValue,
        children: Vec<ExtendedNode>,
    },
    Output {
        for_attr: Option<AttrValue>,
        text: FuzzText,
    },
    Progress {
        value: Option<u8>,
        max: Option<u8>,
    },
    Meter {
        value: Option<u8>,
        min: Option<u8>,
        max: Option<u8>,
    },

    // Void elements
    Br,
    Hr,
    Img {
        src: Option<AttrValue>,
        alt: Option<AttrValue>,
        width: Option<u16>,
        height: Option<u16>,
        loading: Option<ImgLoading>,
    },
    Wbr,

    // Media
    Picture {
        children: Vec<ExtendedNode>,
    },
    Source {
        src: Option<AttrValue>,
        srcset: Option<AttrValue>,
        media: Option<AttrValue>,
        type_attr: Option<AttrValue>,
    },
    Video {
        src: Option<AttrValue>,
        controls: bool,
        autoplay: bool,
        loop_attr: bool,
        muted: bool,
        children: Vec<ExtendedNode>,
    },
    Audio {
        src: Option<AttrValue>,
        controls: bool,
        autoplay: bool,
        loop_attr: bool,
        muted: bool,
        children: Vec<ExtendedNode>,
    },
    Track {
        src: Option<AttrValue>,
        kind: Option<TrackKind>,
        label: Option<AttrValue>,
    },

    // Interactive
    Details {
        open: bool,
        children: Vec<ExtendedNode>,
    },
    Summary {
        text: FuzzText,
    },
    Dialog {
        open: bool,
        children: Vec<ExtendedNode>,
    },

    // Template/Slots (Web Components)
    Template {
        id: Option<AttrValue>,
        children: Vec<ExtendedNode>,
    },
    Slot {
        name: Option<AttrValue>,
    },

    // Custom element (Web Components)
    CustomElement {
        tag: CustomElementTag,
        attrs: Vec<DataAttr>,
        children: Vec<ExtendedNode>,
    },

    // SVG
    Svg(Box<SvgElement>),

    // Figure
    Figure {
        children: Vec<ExtendedNode>,
    },
    Figcaption {
        text: FuzzText,
    },

    // Embedded
    Iframe {
        src: Option<AttrValue>,
        width: Option<u16>,
        height: Option<u16>,
        title: Option<AttrValue>,
    },
    Embed {
        src: Option<AttrValue>,
        type_attr: Option<AttrValue>,
        width: Option<u16>,
        height: Option<u16>,
    },
    Object {
        data: Option<AttrValue>,
        type_attr: Option<AttrValue>,
        width: Option<u16>,
        height: Option<u16>,
        children: Vec<ExtendedNode>,
    },

    // Address
    Address {
        children: Vec<ExtendedNode>,
    },

    // Ruby (for East Asian typography)
    Ruby {
        children: Vec<ExtendedNode>,
    },
    Rt {
        text: FuzzText,
    },
    Rp {
        text: FuzzText,
    },

    // Bidirectional
    Bdo {
        dir: BdoDir,
        text: FuzzText,
    },
    Bdi {
        text: FuzzText,
    },

    // Data
    Data {
        value: AttrValue,
        text: FuzzText,
    },

    // Map/Area
    Map {
        name: AttrValue,
        children: Vec<ExtendedNode>,
    },
    Area {
        shape: Option<AreaShape>,
        coords: Option<AttrValue>,
        href: Option<AttrValue>,
        alt: Option<AttrValue>,
    },

    // Noscript
    Noscript {
        children: Vec<ExtendedNode>,
    },

    // Canvas
    Canvas {
        width: Option<u16>,
        height: Option<u16>,
        children: Vec<ExtendedNode>,
    },
}

// SVG Elements
#[derive(Arbitrary, Debug, Clone)]
enum SvgElement {
    Svg {
        width: Option<SvgLength>,
        height: Option<SvgLength>,
        viewBox: Option<ViewBox>,
        xmlns: bool,
        children: Vec<SvgElement>,
    },
    G {
        id: Option<AttrValue>,
        class: Option<AttrValue>,
        transform: Option<SvgTransform>,
        children: Vec<SvgElement>,
    },
    Defs {
        children: Vec<SvgElement>,
    },
    Symbol {
        id: Option<AttrValue>,
        viewBox: Option<ViewBox>,
        children: Vec<SvgElement>,
    },
    Use {
        href: Option<AttrValue>,
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        width: Option<SvgLength>,
        height: Option<SvgLength>,
    },
    Rect {
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        width: Option<SvgLength>,
        height: Option<SvgLength>,
        rx: Option<SvgLength>,
        ry: Option<SvgLength>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
        stroke_width: Option<SvgLength>,
    },
    Circle {
        cx: Option<SvgLength>,
        cy: Option<SvgLength>,
        r: Option<SvgLength>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
        stroke_width: Option<SvgLength>,
    },
    Ellipse {
        cx: Option<SvgLength>,
        cy: Option<SvgLength>,
        rx: Option<SvgLength>,
        ry: Option<SvgLength>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
    },
    Line {
        x1: Option<SvgLength>,
        y1: Option<SvgLength>,
        x2: Option<SvgLength>,
        y2: Option<SvgLength>,
        stroke: Option<SvgColor>,
        stroke_width: Option<SvgLength>,
    },
    Polyline {
        points: Option<SvgPoints>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
    },
    Polygon {
        points: Option<SvgPoints>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
    },
    Path {
        d: Option<SvgPathData>,
        fill: Option<SvgColor>,
        stroke: Option<SvgColor>,
        stroke_width: Option<SvgLength>,
    },
    Text {
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        fill: Option<SvgColor>,
        font_size: Option<SvgLength>,
        text_anchor: Option<TextAnchor>,
        content: FuzzText,
    },
    Tspan {
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        dx: Option<SvgLength>,
        dy: Option<SvgLength>,
        content: FuzzText,
    },
    Image {
        href: Option<AttrValue>,
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        width: Option<SvgLength>,
        height: Option<SvgLength>,
    },
    ClipPath {
        id: Option<AttrValue>,
        children: Vec<SvgElement>,
    },
    Mask {
        id: Option<AttrValue>,
        children: Vec<SvgElement>,
    },
    LinearGradient {
        id: Option<AttrValue>,
        x1: Option<SvgLength>,
        y1: Option<SvgLength>,
        x2: Option<SvgLength>,
        y2: Option<SvgLength>,
        children: Vec<SvgElement>,
    },
    RadialGradient {
        id: Option<AttrValue>,
        cx: Option<SvgLength>,
        cy: Option<SvgLength>,
        r: Option<SvgLength>,
        children: Vec<SvgElement>,
    },
    Stop {
        offset: Option<SvgLength>,
        stop_color: Option<SvgColor>,
        stop_opacity: Option<f32>,
    },
    Pattern {
        id: Option<AttrValue>,
        width: Option<SvgLength>,
        height: Option<SvgLength>,
        patternUnits: Option<PatternUnits>,
        children: Vec<SvgElement>,
    },
    Filter {
        id: Option<AttrValue>,
        children: Vec<SvgElement>,
    },
    FeGaussianBlur {
        in_attr: Option<FeIn>,
        stdDeviation: Option<f32>,
    },
    FeBlend {
        in_attr: Option<FeIn>,
        in2: Option<FeIn>,
        mode: Option<BlendMode>,
    },
    FeColorMatrix {
        in_attr: Option<FeIn>,
        type_attr: Option<ColorMatrixType>,
        values: Option<AttrValue>,
    },
    FeOffset {
        in_attr: Option<FeIn>,
        dx: Option<SvgLength>,
        dy: Option<SvgLength>,
    },
    FeMerge {
        children: Vec<SvgElement>,
    },
    FeMergeNode {
        in_attr: Option<FeIn>,
    },
    Animate {
        attributeName: Option<AttrValue>,
        from: Option<AttrValue>,
        to: Option<AttrValue>,
        dur: Option<AttrValue>,
        repeatCount: Option<AttrValue>,
    },
    AnimateTransform {
        attributeName: Option<AttrValue>,
        type_attr: Option<TransformType>,
        from: Option<AttrValue>,
        to: Option<AttrValue>,
        dur: Option<AttrValue>,
        repeatCount: Option<AttrValue>,
    },
    ForeignObject {
        x: Option<SvgLength>,
        y: Option<SvgLength>,
        width: Option<SvgLength>,
        height: Option<SvgLength>,
        children: Vec<ExtendedNode>,
    },
    Title {
        text: FuzzText,
    },
    Desc {
        text: FuzzText,
    },
}

// SVG helper types
#[derive(Debug, Clone)]
struct SvgLength(String);

impl<'a> Arbitrary<'a> for SvgLength {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let num = u.int_in_range(0..=1000)?;
        let unit = *u.choose(&["", "px", "%", "em", "rem", "pt", "cm", "mm"])?;
        Ok(SvgLength(format!("{}{}", num, unit)))
    }
}

#[derive(Debug, Clone)]
struct ViewBox {
    min_x: i16,
    min_y: i16,
    width: u16,
    height: u16,
}

impl<'a> Arbitrary<'a> for ViewBox {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(ViewBox {
            min_x: u.int_in_range(-100..=100)?,
            min_y: u.int_in_range(-100..=100)?,
            width: u.int_in_range(1..=2000)?,
            height: u.int_in_range(1..=2000)?,
        })
    }
}

#[derive(Debug, Clone)]
struct SvgTransform(String);

impl<'a> Arbitrary<'a> for SvgTransform {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let transform_type = *u.choose(&["translate", "rotate", "scale", "skewX", "skewY"])?;
        let val1: i16 = u.int_in_range(-500..=500)?;
        let val2: i16 = u.int_in_range(-500..=500)?;

        let s = match transform_type {
            "translate" => format!("translate({}, {})", val1, val2),
            "rotate" => format!("rotate({})", val1),
            "scale" => format!("scale({}, {})", val1 as f32 / 100.0, val2 as f32 / 100.0),
            "skewX" => format!("skewX({})", val1),
            "skewY" => format!("skewY({})", val1),
            _ => "".to_string(),
        };
        Ok(SvgTransform(s))
    }
}

#[derive(Debug, Clone)]
struct SvgColor(String);

impl<'a> Arbitrary<'a> for SvgColor {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let color = if u.ratio(1, 3)? {
            // Named color
            let colors = [
                "red",
                "green",
                "blue",
                "black",
                "white",
                "gray",
                "yellow",
                "orange",
                "purple",
                "pink",
                "cyan",
                "magenta",
                "none",
                "currentColor",
                "transparent",
            ];
            u.choose(&colors)?.to_string()
        } else if u.ratio(1, 2)? {
            // Hex color
            let r: u8 = u.arbitrary()?;
            let g: u8 = u.arbitrary()?;
            let b: u8 = u.arbitrary()?;
            format!("#{:02x}{:02x}{:02x}", r, g, b)
        } else {
            // RGB/RGBA
            let r: u8 = u.arbitrary()?;
            let g: u8 = u.arbitrary()?;
            let b: u8 = u.arbitrary()?;
            if u.ratio(1, 2)? {
                let a: f32 = u.int_in_range(0..=100)? as f32 / 100.0;
                format!("rgba({}, {}, {}, {})", r, g, b, a)
            } else {
                format!("rgb({}, {}, {})", r, g, b)
            }
        };
        Ok(SvgColor(color))
    }
}

#[derive(Debug, Clone)]
struct SvgPoints(String);

impl<'a> Arbitrary<'a> for SvgPoints {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let point_count = u.int_in_range(3..=10)?;
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            let x: i16 = u.int_in_range(0..=500)?;
            let y: i16 = u.int_in_range(0..=500)?;
            points.push(format!("{},{}", x, y));
        }
        Ok(SvgPoints(points.join(" ")))
    }
}

#[derive(Debug, Clone)]
struct SvgPathData(String);

impl<'a> Arbitrary<'a> for SvgPathData {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut d = String::new();
        let cmd_count = u.int_in_range(2..=15)?;

        for i in 0..cmd_count {
            if i == 0 {
                // Start with M
                let x: i16 = u.int_in_range(0..=500)?;
                let y: i16 = u.int_in_range(0..=500)?;
                d.push_str(&format!("M{} {}", x, y));
            } else {
                let cmd = *u.choose(&['L', 'H', 'V', 'C', 'S', 'Q', 'T', 'A', 'Z'])?;
                match cmd {
                    'L' => {
                        let x: i16 = u.int_in_range(0..=500)?;
                        let y: i16 = u.int_in_range(0..=500)?;
                        d.push_str(&format!(" L{} {}", x, y));
                    }
                    'H' => {
                        let x: i16 = u.int_in_range(0..=500)?;
                        d.push_str(&format!(" H{}", x));
                    }
                    'V' => {
                        let y: i16 = u.int_in_range(0..=500)?;
                        d.push_str(&format!(" V{}", y));
                    }
                    'C' => {
                        let vals: Vec<i16> = (0..6)
                            .map(|_| u.int_in_range(0..=500))
                            .collect::<Result<_, _>>()?;
                        d.push_str(&format!(
                            " C{} {} {} {} {} {}",
                            vals[0], vals[1], vals[2], vals[3], vals[4], vals[5]
                        ));
                    }
                    'S' => {
                        let vals: Vec<i16> = (0..4)
                            .map(|_| u.int_in_range(0..=500))
                            .collect::<Result<_, _>>()?;
                        d.push_str(&format!(
                            " S{} {} {} {}",
                            vals[0], vals[1], vals[2], vals[3]
                        ));
                    }
                    'Q' => {
                        let vals: Vec<i16> = (0..4)
                            .map(|_| u.int_in_range(0..=500))
                            .collect::<Result<_, _>>()?;
                        d.push_str(&format!(
                            " Q{} {} {} {}",
                            vals[0], vals[1], vals[2], vals[3]
                        ));
                    }
                    'T' => {
                        let x: i16 = u.int_in_range(0..=500)?;
                        let y: i16 = u.int_in_range(0..=500)?;
                        d.push_str(&format!(" T{} {}", x, y));
                    }
                    'A' => {
                        let rx: u16 = u.int_in_range(1..=100)?;
                        let ry: u16 = u.int_in_range(1..=100)?;
                        let rot: i16 = u.int_in_range(0..=360)?;
                        let large: u8 = u.int_in_range(0..=1)?;
                        let sweep: u8 = u.int_in_range(0..=1)?;
                        let x: i16 = u.int_in_range(0..=500)?;
                        let y: i16 = u.int_in_range(0..=500)?;
                        d.push_str(&format!(
                            " A{} {} {} {} {} {} {}",
                            rx, ry, rot, large, sweep, x, y
                        ));
                    }
                    'Z' => d.push_str(" Z"),
                    _ => {}
                }
            }
        }
        Ok(SvgPathData(d))
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum TextAnchor {
    Start,
    Middle,
    End,
}

impl TextAnchor {
    fn as_str(&self) -> &'static str {
        match self {
            TextAnchor::Start => "start",
            TextAnchor::Middle => "middle",
            TextAnchor::End => "end",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum PatternUnits {
    UserSpaceOnUse,
    ObjectBoundingBox,
}

impl PatternUnits {
    fn as_str(&self) -> &'static str {
        match self {
            PatternUnits::UserSpaceOnUse => "userSpaceOnUse",
            PatternUnits::ObjectBoundingBox => "objectBoundingBox",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum FeIn {
    SourceGraphic,
    SourceAlpha,
    BackgroundImage,
    BackgroundAlpha,
    FillPaint,
    StrokePaint,
}

impl FeIn {
    fn as_str(&self) -> &'static str {
        match self {
            FeIn::SourceGraphic => "SourceGraphic",
            FeIn::SourceAlpha => "SourceAlpha",
            FeIn::BackgroundImage => "BackgroundImage",
            FeIn::BackgroundAlpha => "BackgroundAlpha",
            FeIn::FillPaint => "FillPaint",
            FeIn::StrokePaint => "StrokePaint",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
}

impl BlendMode {
    fn as_str(&self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum ColorMatrixType {
    Matrix,
    Saturate,
    HueRotate,
    LuminanceToAlpha,
}

impl ColorMatrixType {
    fn as_str(&self) -> &'static str {
        match self {
            ColorMatrixType::Matrix => "matrix",
            ColorMatrixType::Saturate => "saturate",
            ColorMatrixType::HueRotate => "hueRotate",
            ColorMatrixType::LuminanceToAlpha => "luminanceToAlpha",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum TransformType {
    Translate,
    Scale,
    Rotate,
    SkewX,
    SkewY,
}

impl TransformType {
    fn as_str(&self) -> &'static str {
        match self {
            TransformType::Translate => "translate",
            TransformType::Scale => "scale",
            TransformType::Rotate => "rotate",
            TransformType::SkewX => "skewX",
            TransformType::SkewY => "skewY",
        }
    }
}

// Custom element tag (must contain hyphen per Web Components spec)
#[derive(Debug, Clone)]
struct CustomElementTag(String);

impl<'a> Arbitrary<'a> for CustomElementTag {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let prefixes = [
            "my", "app", "ui", "x", "custom", "web", "vue", "ng", "react",
        ];
        let suffixes = [
            "component",
            "element",
            "widget",
            "card",
            "button",
            "input",
            "list",
            "item",
            "container",
            "wrapper",
            "header",
            "footer",
            "nav",
            "menu",
            "modal",
            "dialog",
            "tooltip",
            "dropdown",
        ];
        let prefix = *u.choose(&prefixes)?;
        let suffix = *u.choose(&suffixes)?;
        Ok(CustomElementTag(format!("{}-{}", prefix, suffix)))
    }
}

// Data attributes
#[derive(Debug, Clone)]
struct DataAttr {
    name: String,
    value: AttrValue,
}

impl<'a> Arbitrary<'a> for DataAttr {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let names = [
            "data-id",
            "data-index",
            "data-value",
            "data-type",
            "data-state",
            "data-action",
            "data-target",
            "data-toggle",
            "data-dismiss",
            "data-placement",
            "data-trigger",
            "data-content",
            "data-title",
            "data-original-title",
            "data-container",
            "data-delay",
            "data-animation",
            "data-html",
            "data-template",
            "data-selector",
            "data-offset",
            "data-boundary",
            "data-custom",
            "data-test",
            "data-testid",
            "data-cy",
        ];
        let name = u.choose(&names)?.to_string();
        let value = AttrValue::arbitrary(u)?;
        Ok(DataAttr { name, value })
    }
}

// More enum types for attributes
#[derive(Arbitrary, Debug, Clone)]
enum ThScope {
    Row,
    Col,
    Rowgroup,
    Colgroup,
}

impl ThScope {
    fn as_str(&self) -> &'static str {
        match self {
            ThScope::Row => "row",
            ThScope::Col => "col",
            ThScope::Rowgroup => "rowgroup",
            ThScope::Colgroup => "colgroup",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum ATarget {
    Blank,
    Self_,
    Parent,
    Top,
}

impl ATarget {
    fn as_str(&self) -> &'static str {
        match self {
            ATarget::Blank => "_blank",
            ATarget::Self_ => "_self",
            ATarget::Parent => "_parent",
            ATarget::Top => "_top",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum FormMethod {
    Get,
    Post,
    Dialog,
}

impl FormMethod {
    fn as_str(&self) -> &'static str {
        match self {
            FormMethod::Get => "get",
            FormMethod::Post => "post",
            FormMethod::Dialog => "dialog",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum ImgLoading {
    Lazy,
    Eager,
}

impl ImgLoading {
    fn as_str(&self) -> &'static str {
        match self {
            ImgLoading::Lazy => "lazy",
            ImgLoading::Eager => "eager",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum TrackKind {
    Subtitles,
    Captions,
    Descriptions,
    Chapters,
    Metadata,
}

impl TrackKind {
    fn as_str(&self) -> &'static str {
        match self {
            TrackKind::Subtitles => "subtitles",
            TrackKind::Captions => "captions",
            TrackKind::Descriptions => "descriptions",
            TrackKind::Chapters => "chapters",
            TrackKind::Metadata => "metadata",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum BdoDir {
    Ltr,
    Rtl,
}

impl BdoDir {
    fn as_str(&self) -> &'static str {
        match self {
            BdoDir::Ltr => "ltr",
            BdoDir::Rtl => "rtl",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
enum AreaShape {
    Rect,
    Circle,
    Poly,
    Default,
}

impl AreaShape {
    fn as_str(&self) -> &'static str {
        match self {
            AreaShape::Rect => "rect",
            AreaShape::Circle => "circle",
            AreaShape::Poly => "poly",
            AreaShape::Default => "default",
        }
    }
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
    Tel,
    Url,
    Search,
    Date,
    Time,
    DatetimeLocal,
    Month,
    Week,
    Color,
    File,
    Hidden,
    Range,
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
            InputType::Tel => "tel",
            InputType::Url => "url",
            InputType::Search => "search",
            InputType::Date => "date",
            InputType::Time => "time",
            InputType::DatetimeLocal => "datetime-local",
            InputType::Month => "month",
            InputType::Week => "week",
            InputType::Color => "color",
            InputType::File => "file",
            InputType::Hidden => "hidden",
            InputType::Range => "range",
        }
    }
}

#[derive(Arbitrary, Debug, Clone)]
struct FuzzText(
    #[arbitrary(with = |u: &mut arbitrary::Unstructured| {
        let len = u.int_in_range(0..=50)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            let c = match u.int_in_range::<u8>(0..=30)? {
                0..=10 => u.int_in_range(b'a'..=b'z')? as char,
                11..=15 => u.int_in_range(b'A'..=b'Z')? as char,
                16..=20 => u.int_in_range(b'0'..=b'9')? as char,
                21 => '<',
                22 => '>',
                23 => '&',
                24 => '"',
                25 => '\'',
                26 => ' ',
                27 => '\n',
                28 => '\t',
                // Unicode chars
                29 => {
                    let unicode = ['é', 'ñ', 'ü', '中', '日', 'α', 'β', '→', '•'];
                    *u.choose(&unicode)?
                }
                30 => {
                    let emoji = ['😀', '🎉', '🚀', '💯', '❤', '✓', '✗'];
                    *u.choose(&emoji)?
                }
                _ => ' ',
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
        let len = u.int_in_range(0..=30)?;
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            let c = match u.int_in_range::<u8>(0..=20)? {
                0..=10 => u.int_in_range(b'a'..=b'z')? as char,
                11..=13 => u.int_in_range(b'A'..=b'Z')? as char,
                14..=16 => u.int_in_range(b'0'..=b'9')? as char,
                17 => '-',
                18 => '_',
                19 => '/',
                20 => '.',
                _ => unreachable!(),
            };
            s.push(c);
        }
        Ok(s)
    })]
    String,
);

// Helper functions
fn html_escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn bool_attr(name: &str, value: bool) -> String {
    if value {
        format!(" {}", name)
    } else {
        String::new()
    }
}

fn opt_attr(name: &str, value: &Option<AttrValue>) -> String {
    value
        .as_ref()
        .map(|v| format!(" {}=\"{}\"", name, html_escape_attr(&v.0)))
        .unwrap_or_default()
}

fn opt_attr_str(name: &str, value: Option<&str>) -> String {
    value
        .map(|v| format!(" {}=\"{}\"", name, html_escape_attr(v)))
        .unwrap_or_default()
}

fn opt_num_attr<T: std::fmt::Display>(name: &str, value: &Option<T>) -> String {
    value
        .as_ref()
        .map(|v| format!(" {}=\"{}\"", name, v))
        .unwrap_or_default()
}

// SVG to_html implementation
impl SvgElement {
    fn to_html(&self, depth: usize) -> String {
        if depth > 8 {
            return String::new();
        }

        match self {
            SvgElement::Svg {
                width,
                height,
                viewBox,
                xmlns,
                children,
            } => {
                let mut attrs = String::new();
                if *xmlns {
                    attrs.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
                }
                if let Some(w) = width {
                    attrs.push_str(&format!(" width=\"{}\"", w.0));
                }
                if let Some(h) = height {
                    attrs.push_str(&format!(" height=\"{}\"", h.0));
                }
                if let Some(vb) = viewBox {
                    attrs.push_str(&format!(
                        " viewBox=\"{} {} {} {}\"",
                        vb.min_x, vb.min_y, vb.width, vb.height
                    ));
                }
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<svg{}>{}</svg>", attrs, inner)
            }
            SvgElement::G {
                id,
                class,
                transform,
                children,
            } => {
                let mut attrs = String::new();
                attrs.push_str(&opt_attr("id", id));
                attrs.push_str(&opt_attr("class", class));
                if let Some(t) = transform {
                    attrs.push_str(&format!(" transform=\"{}\"", t.0));
                }
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<g{}>{}</g>", attrs, inner)
            }
            SvgElement::Defs { children } => {
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<defs>{}</defs>", inner)
            }
            SvgElement::Symbol {
                id,
                viewBox,
                children,
            } => {
                let mut attrs = opt_attr("id", id);
                if let Some(vb) = viewBox {
                    attrs.push_str(&format!(
                        " viewBox=\"{} {} {} {}\"",
                        vb.min_x, vb.min_y, vb.width, vb.height
                    ));
                }
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<symbol{}>{}</symbol>", attrs, inner)
            }
            SvgElement::Use {
                href,
                x,
                y,
                width,
                height,
            } => {
                let mut attrs = String::new();
                if let Some(h) = href {
                    attrs.push_str(&format!(" href=\"#{}\"", html_escape_attr(&h.0)));
                }
                if let Some(x) = x {
                    attrs.push_str(&format!(" x=\"{}\"", x.0));
                }
                if let Some(y) = y {
                    attrs.push_str(&format!(" y=\"{}\"", y.0));
                }
                if let Some(w) = width {
                    attrs.push_str(&format!(" width=\"{}\"", w.0));
                }
                if let Some(h) = height {
                    attrs.push_str(&format!(" height=\"{}\"", h.0));
                }
                format!("<use{}/>", attrs)
            }
            SvgElement::Rect {
                x,
                y,
                width,
                height,
                rx,
                ry,
                fill,
                stroke,
                stroke_width,
            } => {
                let mut attrs = String::new();
                if let Some(v) = x {
                    attrs.push_str(&format!(" x=\"{}\"", v.0));
                }
                if let Some(v) = y {
                    attrs.push_str(&format!(" y=\"{}\"", v.0));
                }
                if let Some(v) = width {
                    attrs.push_str(&format!(" width=\"{}\"", v.0));
                }
                if let Some(v) = height {
                    attrs.push_str(&format!(" height=\"{}\"", v.0));
                }
                if let Some(v) = rx {
                    attrs.push_str(&format!(" rx=\"{}\"", v.0));
                }
                if let Some(v) = ry {
                    attrs.push_str(&format!(" ry=\"{}\"", v.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                if let Some(sw) = stroke_width {
                    attrs.push_str(&format!(" stroke-width=\"{}\"", sw.0));
                }
                format!("<rect{}/>", attrs)
            }
            SvgElement::Circle {
                cx,
                cy,
                r,
                fill,
                stroke,
                stroke_width,
            } => {
                let mut attrs = String::new();
                if let Some(v) = cx {
                    attrs.push_str(&format!(" cx=\"{}\"", v.0));
                }
                if let Some(v) = cy {
                    attrs.push_str(&format!(" cy=\"{}\"", v.0));
                }
                if let Some(v) = r {
                    attrs.push_str(&format!(" r=\"{}\"", v.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                if let Some(sw) = stroke_width {
                    attrs.push_str(&format!(" stroke-width=\"{}\"", sw.0));
                }
                format!("<circle{}/>", attrs)
            }
            SvgElement::Ellipse {
                cx,
                cy,
                rx,
                ry,
                fill,
                stroke,
            } => {
                let mut attrs = String::new();
                if let Some(v) = cx {
                    attrs.push_str(&format!(" cx=\"{}\"", v.0));
                }
                if let Some(v) = cy {
                    attrs.push_str(&format!(" cy=\"{}\"", v.0));
                }
                if let Some(v) = rx {
                    attrs.push_str(&format!(" rx=\"{}\"", v.0));
                }
                if let Some(v) = ry {
                    attrs.push_str(&format!(" ry=\"{}\"", v.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                format!("<ellipse{}/>", attrs)
            }
            SvgElement::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                stroke_width,
            } => {
                let mut attrs = String::new();
                if let Some(v) = x1 {
                    attrs.push_str(&format!(" x1=\"{}\"", v.0));
                }
                if let Some(v) = y1 {
                    attrs.push_str(&format!(" y1=\"{}\"", v.0));
                }
                if let Some(v) = x2 {
                    attrs.push_str(&format!(" x2=\"{}\"", v.0));
                }
                if let Some(v) = y2 {
                    attrs.push_str(&format!(" y2=\"{}\"", v.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                if let Some(sw) = stroke_width {
                    attrs.push_str(&format!(" stroke-width=\"{}\"", sw.0));
                }
                format!("<line{}/>", attrs)
            }
            SvgElement::Polyline {
                points,
                fill,
                stroke,
            } => {
                let mut attrs = String::new();
                if let Some(p) = points {
                    attrs.push_str(&format!(" points=\"{}\"", p.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                format!("<polyline{}/>", attrs)
            }
            SvgElement::Polygon {
                points,
                fill,
                stroke,
            } => {
                let mut attrs = String::new();
                if let Some(p) = points {
                    attrs.push_str(&format!(" points=\"{}\"", p.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                format!("<polygon{}/>", attrs)
            }
            SvgElement::Path {
                d,
                fill,
                stroke,
                stroke_width,
            } => {
                let mut attrs = String::new();
                if let Some(d) = d {
                    attrs.push_str(&format!(" d=\"{}\"", d.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(s) = stroke {
                    attrs.push_str(&format!(" stroke=\"{}\"", s.0));
                }
                if let Some(sw) = stroke_width {
                    attrs.push_str(&format!(" stroke-width=\"{}\"", sw.0));
                }
                format!("<path{}/>", attrs)
            }
            SvgElement::Text {
                x,
                y,
                fill,
                font_size,
                text_anchor,
                content,
            } => {
                let mut attrs = String::new();
                if let Some(v) = x {
                    attrs.push_str(&format!(" x=\"{}\"", v.0));
                }
                if let Some(v) = y {
                    attrs.push_str(&format!(" y=\"{}\"", v.0));
                }
                if let Some(f) = fill {
                    attrs.push_str(&format!(" fill=\"{}\"", f.0));
                }
                if let Some(fs) = font_size {
                    attrs.push_str(&format!(" font-size=\"{}\"", fs.0));
                }
                if let Some(ta) = text_anchor {
                    attrs.push_str(&format!(" text-anchor=\"{}\"", ta.as_str()));
                }
                format!("<text{}>{}</text>", attrs, html_escape_text(&content.0))
            }
            SvgElement::Tspan {
                x,
                y,
                dx,
                dy,
                content,
            } => {
                let mut attrs = String::new();
                if let Some(v) = x {
                    attrs.push_str(&format!(" x=\"{}\"", v.0));
                }
                if let Some(v) = y {
                    attrs.push_str(&format!(" y=\"{}\"", v.0));
                }
                if let Some(v) = dx {
                    attrs.push_str(&format!(" dx=\"{}\"", v.0));
                }
                if let Some(v) = dy {
                    attrs.push_str(&format!(" dy=\"{}\"", v.0));
                }
                format!("<tspan{}>{}</tspan>", attrs, html_escape_text(&content.0))
            }
            SvgElement::Image {
                href,
                x,
                y,
                width,
                height,
            } => {
                let mut attrs = String::new();
                if let Some(h) = href {
                    attrs.push_str(&format!(" href=\"{}\"", html_escape_attr(&h.0)));
                }
                if let Some(v) = x {
                    attrs.push_str(&format!(" x=\"{}\"", v.0));
                }
                if let Some(v) = y {
                    attrs.push_str(&format!(" y=\"{}\"", v.0));
                }
                if let Some(w) = width {
                    attrs.push_str(&format!(" width=\"{}\"", w.0));
                }
                if let Some(h) = height {
                    attrs.push_str(&format!(" height=\"{}\"", h.0));
                }
                format!("<image{}/>", attrs)
            }
            SvgElement::ClipPath { id, children } => {
                let attrs = opt_attr("id", id);
                let inner: String = children
                    .iter()
                    .take(4)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<clipPath{}>{}</clipPath>", attrs, inner)
            }
            SvgElement::Mask { id, children } => {
                let attrs = opt_attr("id", id);
                let inner: String = children
                    .iter()
                    .take(4)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<mask{}>{}</mask>", attrs, inner)
            }
            SvgElement::LinearGradient {
                id,
                x1,
                y1,
                x2,
                y2,
                children,
            } => {
                let mut attrs = opt_attr("id", id);
                if let Some(v) = x1 {
                    attrs.push_str(&format!(" x1=\"{}\"", v.0));
                }
                if let Some(v) = y1 {
                    attrs.push_str(&format!(" y1=\"{}\"", v.0));
                }
                if let Some(v) = x2 {
                    attrs.push_str(&format!(" x2=\"{}\"", v.0));
                }
                if let Some(v) = y2 {
                    attrs.push_str(&format!(" y2=\"{}\"", v.0));
                }
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<linearGradient{}>{}</linearGradient>", attrs, inner)
            }
            SvgElement::RadialGradient {
                id,
                cx,
                cy,
                r,
                children,
            } => {
                let mut attrs = opt_attr("id", id);
                if let Some(v) = cx {
                    attrs.push_str(&format!(" cx=\"{}\"", v.0));
                }
                if let Some(v) = cy {
                    attrs.push_str(&format!(" cy=\"{}\"", v.0));
                }
                if let Some(v) = r {
                    attrs.push_str(&format!(" r=\"{}\"", v.0));
                }
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<radialGradient{}>{}</radialGradient>", attrs, inner)
            }
            SvgElement::Stop {
                offset,
                stop_color,
                stop_opacity,
            } => {
                let mut attrs = String::new();
                if let Some(o) = offset {
                    attrs.push_str(&format!(" offset=\"{}\"", o.0));
                }
                if let Some(c) = stop_color {
                    attrs.push_str(&format!(" stop-color=\"{}\"", c.0));
                }
                if let Some(op) = stop_opacity {
                    attrs.push_str(&format!(" stop-opacity=\"{}\"", op));
                }
                format!("<stop{}/>", attrs)
            }
            SvgElement::Pattern {
                id,
                width,
                height,
                patternUnits,
                children,
            } => {
                let mut attrs = opt_attr("id", id);
                if let Some(w) = width {
                    attrs.push_str(&format!(" width=\"{}\"", w.0));
                }
                if let Some(h) = height {
                    attrs.push_str(&format!(" height=\"{}\"", h.0));
                }
                if let Some(pu) = patternUnits {
                    attrs.push_str(&format!(" patternUnits=\"{}\"", pu.as_str()));
                }
                let inner: String = children
                    .iter()
                    .take(4)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<pattern{}>{}</pattern>", attrs, inner)
            }
            SvgElement::Filter { id, children } => {
                let attrs = opt_attr("id", id);
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<filter{}>{}</filter>", attrs, inner)
            }
            SvgElement::FeGaussianBlur {
                in_attr,
                stdDeviation,
            } => {
                let mut attrs = String::new();
                if let Some(i) = in_attr {
                    attrs.push_str(&format!(" in=\"{}\"", i.as_str()));
                }
                if let Some(sd) = stdDeviation {
                    attrs.push_str(&format!(" stdDeviation=\"{}\"", sd));
                }
                format!("<feGaussianBlur{}/>", attrs)
            }
            SvgElement::FeBlend { in_attr, in2, mode } => {
                let mut attrs = String::new();
                if let Some(i) = in_attr {
                    attrs.push_str(&format!(" in=\"{}\"", i.as_str()));
                }
                if let Some(i2) = in2 {
                    attrs.push_str(&format!(" in2=\"{}\"", i2.as_str()));
                }
                if let Some(m) = mode {
                    attrs.push_str(&format!(" mode=\"{}\"", m.as_str()));
                }
                format!("<feBlend{}/>", attrs)
            }
            SvgElement::FeColorMatrix {
                in_attr,
                type_attr,
                values,
            } => {
                let mut attrs = String::new();
                if let Some(i) = in_attr {
                    attrs.push_str(&format!(" in=\"{}\"", i.as_str()));
                }
                if let Some(t) = type_attr {
                    attrs.push_str(&format!(" type=\"{}\"", t.as_str()));
                }
                attrs.push_str(&opt_attr("values", values));
                format!("<feColorMatrix{}/>", attrs)
            }
            SvgElement::FeOffset { in_attr, dx, dy } => {
                let mut attrs = String::new();
                if let Some(i) = in_attr {
                    attrs.push_str(&format!(" in=\"{}\"", i.as_str()));
                }
                if let Some(d) = dx {
                    attrs.push_str(&format!(" dx=\"{}\"", d.0));
                }
                if let Some(d) = dy {
                    attrs.push_str(&format!(" dy=\"{}\"", d.0));
                }
                format!("<feOffset{}/>", attrs)
            }
            SvgElement::FeMerge { children } => {
                let inner: String = children
                    .iter()
                    .take(6)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<feMerge>{}</feMerge>", inner)
            }
            SvgElement::FeMergeNode { in_attr } => {
                let mut attrs = String::new();
                if let Some(i) = in_attr {
                    attrs.push_str(&format!(" in=\"{}\"", i.as_str()));
                }
                format!("<feMergeNode{}/>", attrs)
            }
            SvgElement::Animate {
                attributeName,
                from,
                to,
                dur,
                repeatCount,
            } => {
                let mut attrs = String::new();
                attrs.push_str(&opt_attr("attributeName", attributeName));
                attrs.push_str(&opt_attr("from", from));
                attrs.push_str(&opt_attr("to", to));
                attrs.push_str(&opt_attr("dur", dur));
                attrs.push_str(&opt_attr("repeatCount", repeatCount));
                format!("<animate{}/>", attrs)
            }
            SvgElement::AnimateTransform {
                attributeName,
                type_attr,
                from,
                to,
                dur,
                repeatCount,
            } => {
                let mut attrs = String::new();
                attrs.push_str(&opt_attr("attributeName", attributeName));
                if let Some(t) = type_attr {
                    attrs.push_str(&format!(" type=\"{}\"", t.as_str()));
                }
                attrs.push_str(&opt_attr("from", from));
                attrs.push_str(&opt_attr("to", to));
                attrs.push_str(&opt_attr("dur", dur));
                attrs.push_str(&opt_attr("repeatCount", repeatCount));
                format!("<animateTransform{}/>", attrs)
            }
            SvgElement::ForeignObject {
                x,
                y,
                width,
                height,
                children,
            } => {
                let mut attrs = String::new();
                if let Some(v) = x {
                    attrs.push_str(&format!(" x=\"{}\"", v.0));
                }
                if let Some(v) = y {
                    attrs.push_str(&format!(" y=\"{}\"", v.0));
                }
                if let Some(w) = width {
                    attrs.push_str(&format!(" width=\"{}\"", w.0));
                }
                if let Some(h) = height {
                    attrs.push_str(&format!(" height=\"{}\"", h.0));
                }
                let inner: String = children
                    .iter()
                    .take(4)
                    .map(|c| c.to_html(depth + 1))
                    .collect();
                format!("<foreignObject{}>{}</foreignObject>", attrs, inner)
            }
            SvgElement::Title { text } => {
                format!("<title>{}</title>", html_escape_text(&text.0))
            }
            SvgElement::Desc { text } => {
                format!("<desc>{}</desc>", html_escape_text(&text.0))
            }
        }
    }
}

// ExtendedNode to_html implementation
impl ExtendedNode {
    fn to_html(&self, depth: usize) -> String {
        if depth > 6 {
            return String::new();
        }

        let fmt_children = |children: &[ExtendedNode], limit: usize| -> String {
            children
                .iter()
                .take(limit)
                .map(|c| c.to_html(depth + 1))
                .collect()
        };

        let fmt_data_attrs = |attrs: &[DataAttr]| -> String {
            attrs
                .iter()
                .take(5)
                .map(|a| format!(" {}=\"{}\"", a.name, html_escape_attr(&a.value.0)))
                .collect()
        };

        match self {
            ExtendedNode::Text(t) => html_escape_text(&t.0),
            ExtendedNode::Comment(t) => {
                let safe = t.0.replace("--", "- -").trim_end_matches('-').to_string();
                format!("<!--{}-->", safe)
            }
            ExtendedNode::Div {
                class,
                id,
                style,
                data_attrs,
                children,
            } => {
                let mut attrs = String::new();
                attrs.push_str(&opt_attr("class", class));
                attrs.push_str(&opt_attr("id", id));
                attrs.push_str(&opt_attr("style", style));
                attrs.push_str(&fmt_data_attrs(data_attrs));
                format!("<div{}>{}</div>", attrs, fmt_children(children, 5))
            }
            ExtendedNode::P { class, text } => {
                format!(
                    "<p{}>{}</p>",
                    opt_attr("class", class),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::H1 { id, text } => {
                format!(
                    "<h1{}>{}</h1>",
                    opt_attr("id", id),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::H2 { text } => format!("<h2>{}</h2>", html_escape_text(&text.0)),
            ExtendedNode::H3 { text } => format!("<h3>{}</h3>", html_escape_text(&text.0)),
            ExtendedNode::H4 { text } => format!("<h4>{}</h4>", html_escape_text(&text.0)),
            ExtendedNode::H5 { text } => format!("<h5>{}</h5>", html_escape_text(&text.0)),
            ExtendedNode::H6 { text } => format!("<h6>{}</h6>", html_escape_text(&text.0)),
            ExtendedNode::Section { class, children } => {
                format!(
                    "<section{}>{}</section>",
                    opt_attr("class", class),
                    fmt_children(children, 5)
                )
            }
            ExtendedNode::Article { children } => {
                format!("<article>{}</article>", fmt_children(children, 5))
            }
            ExtendedNode::Aside { children } => {
                format!("<aside>{}</aside>", fmt_children(children, 5))
            }
            ExtendedNode::Header { children } => {
                format!("<header>{}</header>", fmt_children(children, 5))
            }
            ExtendedNode::Footer { children } => {
                format!("<footer>{}</footer>", fmt_children(children, 5))
            }
            ExtendedNode::Nav { children } => {
                format!("<nav>{}</nav>", fmt_children(children, 5))
            }
            ExtendedNode::Main { children } => {
                format!("<main>{}</main>", fmt_children(children, 5))
            }
            ExtendedNode::Ul { class, children } => {
                format!(
                    "<ul{}>{}</ul>",
                    opt_attr("class", class),
                    fmt_children(children, 6)
                )
            }
            ExtendedNode::Ol { start, children } => {
                format!(
                    "<ol{}>{}</ol>",
                    opt_num_attr("start", start),
                    fmt_children(children, 6)
                )
            }
            ExtendedNode::Li { class, children } => {
                format!(
                    "<li{}>{}</li>",
                    opt_attr("class", class),
                    fmt_children(children, 4)
                )
            }
            ExtendedNode::Dl { children } => {
                format!("<dl>{}</dl>", fmt_children(children, 10))
            }
            ExtendedNode::Dt { text } => format!("<dt>{}</dt>", html_escape_text(&text.0)),
            ExtendedNode::Dd { text } => format!("<dd>{}</dd>", html_escape_text(&text.0)),
            ExtendedNode::Table { class, children } => {
                format!(
                    "<table{}>{}</table>",
                    opt_attr("class", class),
                    fmt_children(children, 8)
                )
            }
            ExtendedNode::Thead { children } => {
                format!("<thead>{}</thead>", fmt_children(children, 4))
            }
            ExtendedNode::Tbody { children } => {
                format!("<tbody>{}</tbody>", fmt_children(children, 10))
            }
            ExtendedNode::Tfoot { children } => {
                format!("<tfoot>{}</tfoot>", fmt_children(children, 4))
            }
            ExtendedNode::Tr { children } => {
                format!("<tr>{}</tr>", fmt_children(children, 8))
            }
            ExtendedNode::Td {
                colspan,
                rowspan,
                children,
            } => {
                let mut attrs = String::new();
                if let Some(c) = colspan {
                    if *c > 1 {
                        attrs.push_str(&format!(" colspan=\"{}\"", c));
                    }
                }
                if let Some(r) = rowspan {
                    if *r > 1 {
                        attrs.push_str(&format!(" rowspan=\"{}\"", r));
                    }
                }
                format!("<td{}>{}</td>", attrs, fmt_children(children, 3))
            }
            ExtendedNode::Th { scope, text } => {
                let scope_attr = scope
                    .as_ref()
                    .map(|s| format!(" scope=\"{}\"", s.as_str()))
                    .unwrap_or_default();
                format!("<th{}>{}</th>", scope_attr, html_escape_text(&text.0))
            }
            ExtendedNode::Caption { text } => {
                format!("<caption>{}</caption>", html_escape_text(&text.0))
            }
            ExtendedNode::Span { class, text } => {
                format!(
                    "<span{}>{}</span>",
                    opt_attr("class", class),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::A { href, target, text } => {
                let mut attrs = opt_attr("href", href);
                if let Some(t) = target {
                    attrs.push_str(&format!(" target=\"{}\"", t.as_str()));
                }
                format!("<a{}>{}</a>", attrs, html_escape_text(&text.0))
            }
            ExtendedNode::Strong { text } => {
                format!("<strong>{}</strong>", html_escape_text(&text.0))
            }
            ExtendedNode::Em { text } => format!("<em>{}</em>", html_escape_text(&text.0)),
            ExtendedNode::Code { text } => format!("<code>{}</code>", html_escape_text(&text.0)),
            ExtendedNode::Pre { children } => {
                format!("<pre>{}</pre>", fmt_children(children, 3))
            }
            ExtendedNode::Blockquote { cite, children } => {
                format!(
                    "<blockquote{}>{}</blockquote>",
                    opt_attr("cite", cite),
                    fmt_children(children, 4)
                )
            }
            ExtendedNode::Q { text } => format!("<q>{}</q>", html_escape_text(&text.0)),
            ExtendedNode::Abbr { title, text } => {
                format!(
                    "<abbr{}>{}</abbr>",
                    opt_attr("title", title),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Time { datetime, text } => {
                format!(
                    "<time{}>{}</time>",
                    opt_attr("datetime", datetime),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Mark { text } => format!("<mark>{}</mark>", html_escape_text(&text.0)),
            ExtendedNode::Small { text } => format!("<small>{}</small>", html_escape_text(&text.0)),
            ExtendedNode::Sub { text } => format!("<sub>{}</sub>", html_escape_text(&text.0)),
            ExtendedNode::Sup { text } => format!("<sup>{}</sup>", html_escape_text(&text.0)),
            ExtendedNode::Kbd { text } => format!("<kbd>{}</kbd>", html_escape_text(&text.0)),
            ExtendedNode::Samp { text } => format!("<samp>{}</samp>", html_escape_text(&text.0)),
            ExtendedNode::Var { text } => format!("<var>{}</var>", html_escape_text(&text.0)),
            ExtendedNode::Cite { text } => format!("<cite>{}</cite>", html_escape_text(&text.0)),
            ExtendedNode::Dfn { text } => format!("<dfn>{}</dfn>", html_escape_text(&text.0)),
            ExtendedNode::Del { text } => format!("<del>{}</del>", html_escape_text(&text.0)),
            ExtendedNode::Ins { text } => format!("<ins>{}</ins>", html_escape_text(&text.0)),
            ExtendedNode::S { text } => format!("<s>{}</s>", html_escape_text(&text.0)),
            ExtendedNode::U { text } => format!("<u>{}</u>", html_escape_text(&text.0)),
            ExtendedNode::B { text } => format!("<b>{}</b>", html_escape_text(&text.0)),
            ExtendedNode::I { text } => format!("<i>{}</i>", html_escape_text(&text.0)),
            ExtendedNode::Form {
                action,
                method,
                children,
            } => {
                let mut attrs = opt_attr("action", action);
                if let Some(m) = method {
                    attrs.push_str(&format!(" method=\"{}\"", m.as_str()));
                }
                format!("<form{}>{}</form>", attrs, fmt_children(children, 8))
            }
            ExtendedNode::Button {
                type_attr,
                disabled,
                text,
            } => {
                let mut attrs = String::new();
                if let Some(t) = type_attr {
                    attrs.push_str(&format!(" type=\"{}\"", t.as_str()));
                }
                attrs.push_str(&bool_attr("disabled", *disabled));
                format!("<button{}>{}</button>", attrs, html_escape_text(&text.0))
            }
            ExtendedNode::Input {
                type_attr,
                name,
                value,
                placeholder,
                disabled,
                readonly,
                required,
            } => {
                let mut attrs = String::new();
                if let Some(t) = type_attr {
                    attrs.push_str(&format!(" type=\"{}\"", t.as_str()));
                }
                attrs.push_str(&opt_attr("name", name));
                attrs.push_str(&opt_attr("value", value));
                attrs.push_str(&opt_attr("placeholder", placeholder));
                attrs.push_str(&bool_attr("disabled", *disabled));
                attrs.push_str(&bool_attr("readonly", *readonly));
                attrs.push_str(&bool_attr("required", *required));
                format!("<input{}>", attrs)
            }
            ExtendedNode::Label { for_attr, text } => {
                format!(
                    "<label{}>{}</label>",
                    opt_attr("for", for_attr),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Select {
                name,
                multiple,
                disabled,
                children,
            } => {
                let mut attrs = opt_attr("name", name);
                attrs.push_str(&bool_attr("multiple", *multiple));
                attrs.push_str(&bool_attr("disabled", *disabled));
                format!("<select{}>{}</select>", attrs, fmt_children(children, 10))
            }
            ExtendedNode::Option {
                value,
                selected,
                disabled,
                text,
            } => {
                let mut attrs = opt_attr("value", value);
                attrs.push_str(&bool_attr("selected", *selected));
                attrs.push_str(&bool_attr("disabled", *disabled));
                format!("<option{}>{}</option>", attrs, html_escape_text(&text.0))
            }
            ExtendedNode::Optgroup {
                label,
                disabled,
                children,
            } => {
                let mut attrs = format!(" label=\"{}\"", html_escape_attr(&label.0));
                attrs.push_str(&bool_attr("disabled", *disabled));
                format!(
                    "<optgroup{}>{}</optgroup>",
                    attrs,
                    fmt_children(children, 10)
                )
            }
            ExtendedNode::Textarea {
                name,
                rows,
                cols,
                placeholder,
                disabled,
                readonly,
                text,
            } => {
                let mut attrs = opt_attr("name", name);
                attrs.push_str(&opt_num_attr("rows", rows));
                attrs.push_str(&opt_num_attr("cols", cols));
                attrs.push_str(&opt_attr("placeholder", placeholder));
                attrs.push_str(&bool_attr("disabled", *disabled));
                attrs.push_str(&bool_attr("readonly", *readonly));
                format!(
                    "<textarea{}>{}</textarea>",
                    attrs,
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Fieldset { disabled, children } => {
                format!(
                    "<fieldset{}>{}</fieldset>",
                    bool_attr("disabled", *disabled),
                    fmt_children(children, 6)
                )
            }
            ExtendedNode::Legend { text } => {
                format!("<legend>{}</legend>", html_escape_text(&text.0))
            }
            ExtendedNode::Datalist { id, children } => {
                format!(
                    "<datalist id=\"{}\">{}</datalist>",
                    html_escape_attr(&id.0),
                    fmt_children(children, 10)
                )
            }
            ExtendedNode::Output { for_attr, text } => {
                format!(
                    "<output{}>{}</output>",
                    opt_attr("for", for_attr),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Progress { value, max } => {
                let mut attrs = opt_num_attr("value", value);
                attrs.push_str(&opt_num_attr("max", max));
                format!("<progress{}></progress>", attrs)
            }
            ExtendedNode::Meter { value, min, max } => {
                let mut attrs = opt_num_attr("value", value);
                attrs.push_str(&opt_num_attr("min", min));
                attrs.push_str(&opt_num_attr("max", max));
                format!("<meter{}></meter>", attrs)
            }
            ExtendedNode::Br => "<br>".to_string(),
            ExtendedNode::Hr => "<hr>".to_string(),
            ExtendedNode::Img {
                src,
                alt,
                width,
                height,
                loading,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&opt_attr("alt", alt));
                attrs.push_str(&opt_num_attr("width", width));
                attrs.push_str(&opt_num_attr("height", height));
                if let Some(l) = loading {
                    attrs.push_str(&format!(" loading=\"{}\"", l.as_str()));
                }
                format!("<img{}>", attrs)
            }
            ExtendedNode::Wbr => "<wbr>".to_string(),
            ExtendedNode::Picture { children } => {
                format!("<picture>{}</picture>", fmt_children(children, 5))
            }
            ExtendedNode::Source {
                src,
                srcset,
                media,
                type_attr,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&opt_attr("srcset", srcset));
                attrs.push_str(&opt_attr("media", media));
                attrs.push_str(&opt_attr("type", type_attr));
                format!("<source{}>", attrs)
            }
            ExtendedNode::Video {
                src,
                controls,
                autoplay,
                loop_attr,
                muted,
                children,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&bool_attr("controls", *controls));
                attrs.push_str(&bool_attr("autoplay", *autoplay));
                attrs.push_str(&bool_attr("loop", *loop_attr));
                attrs.push_str(&bool_attr("muted", *muted));
                format!("<video{}>{}</video>", attrs, fmt_children(children, 4))
            }
            ExtendedNode::Audio {
                src,
                controls,
                autoplay,
                loop_attr,
                muted,
                children,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&bool_attr("controls", *controls));
                attrs.push_str(&bool_attr("autoplay", *autoplay));
                attrs.push_str(&bool_attr("loop", *loop_attr));
                attrs.push_str(&bool_attr("muted", *muted));
                format!("<audio{}>{}</audio>", attrs, fmt_children(children, 4))
            }
            ExtendedNode::Track { src, kind, label } => {
                let mut attrs = opt_attr("src", src);
                if let Some(k) = kind {
                    attrs.push_str(&format!(" kind=\"{}\"", k.as_str()));
                }
                attrs.push_str(&opt_attr("label", label));
                format!("<track{}>", attrs)
            }
            ExtendedNode::Details { open, children } => {
                format!(
                    "<details{}>{}</details>",
                    bool_attr("open", *open),
                    fmt_children(children, 4)
                )
            }
            ExtendedNode::Summary { text } => {
                format!("<summary>{}</summary>", html_escape_text(&text.0))
            }
            ExtendedNode::Dialog { open, children } => {
                format!(
                    "<dialog{}>{}</dialog>",
                    bool_attr("open", *open),
                    fmt_children(children, 4)
                )
            }
            ExtendedNode::Template { id, children } => {
                format!(
                    "<template{}>{}</template>",
                    opt_attr("id", id),
                    fmt_children(children, 4)
                )
            }
            ExtendedNode::Slot { name } => {
                format!("<slot{}>", opt_attr("name", name))
            }
            ExtendedNode::CustomElement {
                tag,
                attrs,
                children,
            } => {
                let attrs_str = fmt_data_attrs(attrs);
                format!(
                    "<{}{}>{}</{}>",
                    tag.0,
                    attrs_str,
                    fmt_children(children, 4),
                    tag.0
                )
            }
            ExtendedNode::Svg(svg) => svg.to_html(depth),
            ExtendedNode::Figure { children } => {
                format!("<figure>{}</figure>", fmt_children(children, 4))
            }
            ExtendedNode::Figcaption { text } => {
                format!("<figcaption>{}</figcaption>", html_escape_text(&text.0))
            }
            ExtendedNode::Iframe {
                src,
                width,
                height,
                title,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&opt_num_attr("width", width));
                attrs.push_str(&opt_num_attr("height", height));
                attrs.push_str(&opt_attr("title", title));
                format!("<iframe{}></iframe>", attrs)
            }
            ExtendedNode::Embed {
                src,
                type_attr,
                width,
                height,
            } => {
                let mut attrs = opt_attr("src", src);
                attrs.push_str(&opt_attr("type", type_attr));
                attrs.push_str(&opt_num_attr("width", width));
                attrs.push_str(&opt_num_attr("height", height));
                format!("<embed{}>", attrs)
            }
            ExtendedNode::Object {
                data,
                type_attr,
                width,
                height,
                children,
            } => {
                let mut attrs = opt_attr("data", data);
                attrs.push_str(&opt_attr("type", type_attr));
                attrs.push_str(&opt_num_attr("width", width));
                attrs.push_str(&opt_num_attr("height", height));
                format!("<object{}>{}</object>", attrs, fmt_children(children, 2))
            }
            ExtendedNode::Address { children } => {
                format!("<address>{}</address>", fmt_children(children, 4))
            }
            ExtendedNode::Ruby { children } => {
                format!("<ruby>{}</ruby>", fmt_children(children, 4))
            }
            ExtendedNode::Rt { text } => format!("<rt>{}</rt>", html_escape_text(&text.0)),
            ExtendedNode::Rp { text } => format!("<rp>{}</rp>", html_escape_text(&text.0)),
            ExtendedNode::Bdo { dir, text } => {
                format!(
                    "<bdo dir=\"{}\">{}</bdo>",
                    dir.as_str(),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Bdi { text } => format!("<bdi>{}</bdi>", html_escape_text(&text.0)),
            ExtendedNode::Data { value, text } => {
                format!(
                    "<data value=\"{}\">{}</data>",
                    html_escape_attr(&value.0),
                    html_escape_text(&text.0)
                )
            }
            ExtendedNode::Map { name, children } => {
                format!(
                    "<map name=\"{}\">{}</map>",
                    html_escape_attr(&name.0),
                    fmt_children(children, 6)
                )
            }
            ExtendedNode::Area {
                shape,
                coords,
                href,
                alt,
            } => {
                let mut attrs = String::new();
                if let Some(s) = shape {
                    attrs.push_str(&format!(" shape=\"{}\"", s.as_str()));
                }
                attrs.push_str(&opt_attr("coords", coords));
                attrs.push_str(&opt_attr("href", href));
                attrs.push_str(&opt_attr("alt", alt));
                format!("<area{}>", attrs)
            }
            ExtendedNode::Noscript { children } => {
                format!("<noscript>{}</noscript>", fmt_children(children, 4))
            }
            ExtendedNode::Canvas {
                width,
                height,
                children,
            } => {
                let mut attrs = opt_num_attr("width", width);
                attrs.push_str(&opt_num_attr("height", height));
                format!("<canvas{}>{}</canvas>", attrs, fmt_children(children, 2))
            }
        }
    }
}

// =============================================================================
// FUZZ INPUT - combines both modes
// =============================================================================

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
enum FuzzMode {
    /// Extended realistic mode with all HTML/SVG elements
    Extended {
        old_doctype: Doctype,
        old: Vec<ExtendedNode>,
        new_doctype: Doctype,
        new: Vec<ExtendedNode>,
    },
    /// Chaos mode with arbitrary tag/attr names
    Chaos {
        old_doctype: Doctype,
        old: Vec<ChaosNode>,
        new_doctype: Doctype,
        new: Vec<ChaosNode>,
    },
    /// Mixed: old is extended, new has chaos elements
    Mixed {
        old_doctype: Doctype,
        old: Vec<ExtendedNode>,
        new_doctype: Doctype,
        new: Vec<ChaosNode>,
    },
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    mode: FuzzMode,
    add_invalid_nesting: bool,
}

fn extended_nodes_to_html(
    nodes: &[ExtendedNode],
    doctype: &Doctype,
    add_invalid_nesting: bool,
) -> String {
    let mut inner: String = nodes.iter().take(6).map(|n| n.to_html(0)).collect();

    if add_invalid_nesting && !inner.is_empty() {
        inner = format!(
            "<p>Unclosed paragraph{}<div>Inside P which browsers will auto-close</div>",
            inner
        );
        inner.push_str("<span>Unclosed span<div>Block in span</div>");
    }

    format!("{}<html><body>{}</body></html>", doctype.as_str(), inner)
}

fn chaos_nodes_to_html(
    nodes: &[ChaosNode],
    doctype: &Doctype,
    add_invalid_nesting: bool,
) -> String {
    let mut inner: String = nodes.iter().take(6).map(|n| n.to_html(0)).collect();

    if add_invalid_nesting && !inner.is_empty() {
        inner = format!(
            "<p>Unclosed paragraph{}<div>Inside P which browsers will auto-close</div>",
            inner
        );
        inner.push_str("<span>Unclosed span<div>Block in span</div>");
    }

    format!("{}<html><body>{}</body></html>", doctype.as_str(), inner)
}

fuzz_target!(|input: FuzzInput| {
    let (old_html, new_html) = match &input.mode {
        FuzzMode::Extended {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            extended_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            extended_nodes_to_html(new, new_doctype, false),
        ),
        FuzzMode::Chaos {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            chaos_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            chaos_nodes_to_html(new, new_doctype, false),
        ),
        FuzzMode::Mixed {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            extended_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            chaos_nodes_to_html(new, new_doctype, false),
        ),
    };

    let patches = hotmeal::diff_html(&old_html, &new_html).expect("diff failed");
    let mut doc = hotmeal::parse(&old_html);
    doc.apply_patches(patches.clone()).expect("apply failed");

    let result = doc.to_html_without_doctype();
    let expected_doc = hotmeal::parse(&new_html);
    let expected = expected_doc.to_html_without_doctype();

    assert_eq!(
        result, expected,
        "Roundtrip failed!\nOld: {}\nNew: {}\nPatches: {:?}",
        old_html, new_html, patches
    );
});
