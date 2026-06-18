use hotmeal::{Document, NodeId, NodeKind, Stem, StrTendril, parse};

const XXLARGE_HTML: &str = include_str!("../tests/fixtures/xxl.html");

fn is_borrowed(stem: &Stem) -> bool {
    matches!(stem, Stem::Borrowed(_))
}

fn count_stems(doc: &Document, input: &str) -> (usize, usize, Vec<String>) {
    let mut borrowed = 0usize;
    let mut owned = 0usize;
    let mut owned_samples: Vec<String> = Vec::new();

    let input_start = input.as_ptr() as usize;
    let input_end = input_start + input.len();

    fn visit(
        doc: &Document,
        node_id: NodeId,
        borrowed: &mut usize,
        owned: &mut usize,
        owned_samples: &mut Vec<String>,
        input_start: usize,
        input_end: usize,
    ) {
        let node = doc.get(node_id);
        match &node.kind {
            NodeKind::Text(stem) | NodeKind::Comment(stem) => {
                if is_borrowed(stem) {
                    *borrowed += 1;
                } else {
                    *owned += 1;
                    if owned_samples.len() < 5 {
                        let s = stem.as_str();
                        let ptr = s.as_ptr() as usize;
                        owned_samples.push(format!(
                            "text {:?} ptr={:#x} (input={:#x}..{:#x})",
                            if s.len() > 30 { &s[..30] } else { s },
                            ptr,
                            input_start,
                            input_end
                        ));
                    }
                }
            }
            NodeKind::Element(elem) => {
                for (_name, value) in &elem.attrs {
                    if is_borrowed(value) {
                        *borrowed += 1;
                    } else {
                        *owned += 1;
                        if owned_samples.len() < 5 {
                            let s = value.as_str();
                            let ptr = s.as_ptr() as usize;
                            owned_samples.push(format!(
                                "attr {:?} ptr={:#x} (input={:#x}..{:#x})",
                                if s.len() > 30 { &s[..30] } else { s },
                                ptr,
                                input_start,
                                input_end
                            ));
                        }
                    }
                }
            }
            _ => {}
        }

        for child in node_id.children(&doc.arena) {
            visit(
                doc,
                child,
                borrowed,
                owned,
                owned_samples,
                input_start,
                input_end,
            );
        }
    }

    visit(
        doc,
        doc.root,
        &mut borrowed,
        &mut owned,
        &mut owned_samples,
        input_start,
        input_end,
    );
    (borrowed, owned, owned_samples)
}

fn main() {
    let tendril = StrTendril::from(XXLARGE_HTML);

    // Print tendril buffer info
    let tendril_str: &str = tendril.as_ref();
    println!(
        "Input tendril ptr: {:#x}..{:#x}",
        tendril_str.as_ptr() as usize,
        tendril_str.as_ptr() as usize + tendril_str.len()
    );
    println!(
        "Static str ptr: {:#x}..{:#x}",
        XXLARGE_HTML.as_ptr() as usize,
        XXLARGE_HTML.as_ptr() as usize + XXLARGE_HTML.len()
    );

    let doc = parse(&tendril);

    let (borrowed, owned, samples) = count_stems(&doc, tendril_str);
    let total = borrowed + owned;
    let pct = if total > 0 {
        (borrowed as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    println!("\nBorrowed: {}", borrowed);
    println!("Owned:    {}", owned);
    println!("Total:    {}", total);
    println!("Zero-copy: {:.1}%", pct);

    if !samples.is_empty() {
        println!("\nFirst {} owned samples:", samples.len());
        for s in &samples {
            println!("  {}", s);
        }
    }
}
