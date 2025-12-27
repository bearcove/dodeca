use pikru::parse::parse;
use pikru::render::render;

fn main() {
    tracing_subscriber::fmt::init();
    
    let code = r#"
linerad = 10px
line right linerad then up linerad
"#;
    
    match parse(code) {
        Ok(program) => {
            match render(&program) {
                Ok(svg) => {
                    println!("\nSVG output:");
                    println!("{}", svg);
                }
                Err(e) => println!("Render error: {:?}", e),
            }
        }
        Err(e) => println!("Parse error: {:?}", e),
    }
}
