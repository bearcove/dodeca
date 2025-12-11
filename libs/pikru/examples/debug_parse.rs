use pest::Parser;
use pikru::{PikchrParser, Rule};

fn main() {
    let input = r#"arrow from C0 to C1 chop color red"#;

    println!("Parsing: {}", input);
    println!();

    match PikchrParser::parse(Rule::statement, input) {
        Ok(pairs) => {
            println!(
                "{}",
                pest_ascii_tree::into_ascii_tree(pairs.clone()).unwrap()
            );
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
        }
    }
}
