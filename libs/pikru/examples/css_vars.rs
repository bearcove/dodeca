use pikru::render::{render_with_options, RenderOptions};

fn main() {
    let source = r#"
box "Hello" fill red
arrow
box "World" fill blue
text "Red and Blue" at (0.5, -0.5) color green
"#;

    // Parse and render
    let program = pikru::parse::parse(source).expect("parse failed");

    // Render with CSS variables
    let options = RenderOptions { css_variables: true };
    let svg = render_with_options(&program, &options).expect("render failed");

    println!("{}", svg);
}
