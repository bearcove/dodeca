use facet_svg::{Svg, SvgNode, facet_xml};

fn main() {
    let input = std::fs::read_to_string("vendor/pikchr-c/tests/test03.pikchr").unwrap();
    let rust_svg = pikru::pikchr(&input).unwrap();

    println!("SVG length: {}", rust_svg.len());

    // Try parsing
    match facet_xml::from_str::<Svg>(&rust_svg) {
        Ok(doc) => {
            let count = count_elements(&doc.children);
            println!("Elements found: {}", count);
        }
        Err(e) => println!("Parse error: {}", e),
    }
}

fn count_elements(children: &[SvgNode]) -> usize {
    let mut count = 0;
    for child in children {
        match child {
            SvgNode::G(g) => count += count_elements(&g.children),
            SvgNode::Defs(d) => count += count_elements(&d.children),
            SvgNode::Style(_) => {}
            SvgNode::Rect(_)
            | SvgNode::Circle(_)
            | SvgNode::Ellipse(_)
            | SvgNode::Line(_)
            | SvgNode::Path(_)
            | SvgNode::Polygon(_)
            | SvgNode::Polyline(_)
            | SvgNode::Text(_)
            | SvgNode::Use(_)
            | SvgNode::Image(_)
            | SvgNode::Title(_)
            | SvgNode::Desc(_) => count += 1,
            SvgNode::Symbol(symbol) => {
                count += 1;
                count += count_elements(&symbol.children);
            }
        }
    }
    count
}
