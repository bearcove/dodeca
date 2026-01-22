// =============================================================================
// Tests
// =============================================================================

use hotmeal::{FlowContent, PhrasingContent, UlContent, parse};

#[test]
fn test_valid_ul() {
    let html = "<html><body><ul><li>Item A</li><li>Item B</li></ul></body></html>";
    let parsed = parse(html);

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

    println!("=== Valid UL ===");
    println!("Parsed successfully!");
}

#[test]
fn test_ul_with_whitespace() {
    let html = "<html><body><ul>\n    <li>Item A</li>\n    <li>Item B</li>\n</ul></body></html>";
    let parsed = parse(html);

    let body = parsed.body.as_ref().expect("should have body");
    assert_eq!(body.children.len(), 1);

    if let FlowContent::Ul(ul) = &body.children[0] {
        // Should have: text, li, text, li, text (5 children)
        println!("UL has {} children", ul.children.len());
        for (i, child) in ul.children.iter().enumerate() {
            match child {
                UlContent::Text(t) => println!("  [{}] Text({:?})", i, t),
                UlContent::Li(li) => println!("  [{}] Li({} children)", i, li.children.len()),
            }
        }
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
    let html = "<html><body><tr><td>cell</td></tr></body></html>";
    let parsed = parse(html);

    let body = parsed.body.as_ref().expect("should have body");
    println!("=== TR outside TABLE ===");
    println!("Body has {} children", body.children.len());
    for (i, child) in body.children.iter().enumerate() {
        match child {
            FlowContent::Text(t) => println!("  [{}] Text({:?})", i, t),
            FlowContent::Table(table) => {
                println!("  [{}] Table({} children)", i, table.children.len())
            }
            FlowContent::Custom(c) => println!("  [{}] Custom({})", i, c.tag),
            _ => println!("  [{}] Other", i),
        }
    }

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
    let html = "<html><body><p>outer<p>inner</p></p></body></html>";
    let parsed = parse(html);

    let body = parsed.body.as_ref().expect("should have body");
    println!("=== P in P ===");
    println!("Body has {} children", body.children.len());

    // Browser creates: <p>outer</p><p>inner</p><p></p>
    // So we should have 3 children
    for (i, child) in body.children.iter().enumerate() {
        match child {
            FlowContent::P(p) => {
                let text: String = p
                    .children
                    .iter()
                    .filter_map(|c| {
                        if let PhrasingContent::Text(t) = c {
                            Some(t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                println!("  [{}] P(text={:?})", i, text);
            }
            FlowContent::Text(t) => println!("  [{}] Text({:?})", i, t),
            _ => println!("  [{}] Other", i),
        }
    }
}
