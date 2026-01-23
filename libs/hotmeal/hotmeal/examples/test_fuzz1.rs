// Minimal reproduction of fuzz seed 1 failure
use hotmeal::{StrTendril, parse};

fn main() {
    // The specific problematic part - strong with section inside
    let html = StrTendril::from(
        r##"<html><body><p>First paragraph with <strong>greater&gt;than<section>line
break</section><svg width="29" height="21"><circle cx="50" cy="50" r="21" fill="none"></circle></svg></strong> and  text.</p></body></html>"##,
    );

    println!("=== Input HTML ===");
    println!("{:?}\n", html.as_ref());

    println!("=== What html5ever produces ===");
    let parsed = parse(&html);
    let output = parsed.to_html();
    println!("{}\n", output);

    // Key question: is <svg> inside <strong> or not?
    if output.contains("<strong><svg") {
        println!("html5ever: SVG IS inside strong");
    } else if output.contains("</section><svg") {
        println!("html5ever: SVG is NOT inside strong (outside section)");
    } else {
        println!("html5ever: Unknown structure");
    }
}
