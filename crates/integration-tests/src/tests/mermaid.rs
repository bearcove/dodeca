use super::*;

pub fn mermaid_flowchart_rendered_to_svg() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // Mermaid is now client-side: server emits <pre class="mermaid"> inside an opaque wrapper
    html.assert_contains("data-hotmeal-opaque=\"mermaid\"");
    html.assert_contains("<pre class=\"mermaid\">");

    // The mermaid source should be present (HTML-escaped) for client-side rendering
    html.assert_contains("Start");
    html.assert_contains("Decision");
    html.assert_contains("Do Something");
    html.assert_contains("End");
}

pub fn mermaid_sequence_diagram_rendered() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // Sequence diagram participants should be present in the pre block
    html.assert_contains("Alice");
    html.assert_contains("Bob");

    // The messages should be present
    html.assert_contains("Hello Bob!");
    html.assert_contains("Hi Alice!");
}

pub fn mermaid_no_raw_code_blocks() {
    let site = TestSite::new("sample-site");

    let html = site.get("/guide/mermaid-test/");
    html.assert_ok();

    // The markdown fenced block markers should NOT appear in output
    html.assert_not_contains("```mermaid");

    // The mermaid.js script should be injected for client-side rendering
    html.assert_contains("mermaid");
}
