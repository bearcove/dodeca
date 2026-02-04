use super::*;

pub fn mermaid_flowchart_rendered_to_svg() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // Mermaid renders diagrams to SVG
    html.assert_contains("<svg");
    html.assert_contains("</svg>");

    // Flowchart should have the nodes we defined
    html.assert_contains("Start");
    html.assert_contains("Decision");
    html.assert_contains("Do Something");
    html.assert_contains("End");
}

pub fn mermaid_sequence_diagram_rendered() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // Sequence diagram participants should be present
    html.assert_contains("Alice");
    html.assert_contains("Bob");

    // The messages should be rendered
    html.assert_contains("Hello Bob!");
    html.assert_contains("Hi Alice!");
}

pub fn mermaid_no_raw_code_blocks() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // The raw mermaid source should NOT appear in output
    // (it should be replaced with rendered SVG)
    html.assert_not_contains("```mermaid");
    html.assert_not_contains("flowchart LR");
    html.assert_not_contains("sequenceDiagram");
    html.assert_not_contains("participant A as Alice");
}
