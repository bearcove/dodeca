use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "pikchr.pest"]
pub struct PikchrParser;

/// Render pikchr source to SVG.
///
/// Returns the SVG string on success, or an error with diagnostics.
pub fn pikchr(source: &str) -> Result<String, miette::Report> {
    // Parse the input
    let pairs = PikchrParser::parse(Rule::program, source)
        .map_err(|e| miette::miette!("Parse error: {}", e))?;

    // For now, just dump the parse tree to prove parsing works
    let mut output = String::new();
    output.push_str("<!-- Parse tree:\n");
    for pair in pairs {
        output.push_str(&format!("{:#?}\n", pair));
    }
    output.push_str("-->\n");

    // TODO: Build AST from parse tree
    // TODO: Evaluate/render to SVG

    Err(miette::miette!("SVG rendering not yet implemented"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_box() {
        let input = r#"box "Hello""#;
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_arrow() {
        let input = "arrow";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_labeled() {
        let input = "A: box";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_multiple_statements() {
        let input = r#"
            box "One"
            arrow
            box "Two"
        "#;
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_sublist() {
        let input = r#"
            A: [
                box "inner"
                arrow
            ]
        "#;
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_position_expr() {
        let input = "box at (1, 2)";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_variable_assignment() {
        let input = "$x = 10";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_dollar_one() {
        let input = "$one = 1.0";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_dot_x() {
        // Test C4.x style access
        let input = "box at C4.x, C4.y";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_expr_edgept() {
        // Test "expr ne of position" style
        let input = "circle at 1 ne of C2";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_paren_expr_edgept() {
        // Test "(expr) ne of position" style - this is the failing case
        let input = "circle at (1+2) ne of C2";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_assert_objects() {
        let input = "assert( previous == last arrow )";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_last_arrow() {
        // Test that "last arrow" parses as an object reference
        let input = "box at last arrow";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_nth_rule() {
        // Direct test of nth parsing
        let input = "last arrow";
        let result = PikchrParser::parse(Rule::nth, input);
        assert!(result.is_ok(), "Failed to parse nth: {:?}", result.err());
    }

    #[test]
    fn parse_position_last_arrow() {
        // "last arrow" is a position (via nth/object/place), not an expr
        let input = "last arrow";
        let result = PikchrParser::parse(Rule::position, input);
        assert!(result.is_ok(), "Failed to parse position: {:?}", result.err());
    }

    #[test]
    fn parse_assert_stmt() {
        // Test assert statement
        let input = "assert( previous == last arrow )";
        let result = PikchrParser::parse(Rule::assert_stmt, input);
        assert!(result.is_ok(), "Failed to parse assert_stmt: {:?}", result.err());
    }

    #[test]
    fn parse_position_just_last() {
        // "last" alone is a position (via nth/object/place), not an expr
        let input = "last";
        let result = PikchrParser::parse(Rule::position, input);
        println!("Result for 'last' as position: {:?}", result);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_start_of() {
        let input = "AS: start of last arrow";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_place_edgept() {
        let input = "start of last arrow";
        let result = PikchrParser::parse(Rule::place, input);
        assert!(result.is_ok(), "Failed to parse place: {:?}", result.err());
    }

    #[test]
    fn parse_edgept_start() {
        let input = "start";
        let result = PikchrParser::parse(Rule::EDGEPT, input);
        assert!(result.is_ok(), "Failed to parse EDGEPT: {:?}", result.err());
    }

    #[test]
    fn parse_place_simple() {
        // Simplest form: EDGEPT of object
        let input = "n of C2";
        let result = PikchrParser::parse(Rule::place, input);
        assert!(result.is_ok(), "Failed to parse place: {:?}", result.err());
    }

    #[test]
    fn parse_place_start_of_c2() {
        let input = "start of C2";
        let result = PikchrParser::parse(Rule::place, input);
        assert!(result.is_ok(), "Failed to parse place: {:?}", result.err());
    }

    #[test]
    fn parse_test01() {
        let input = include_str!("../../pikchr/tests/test01.pikchr");
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse test01.pikchr: {:?}", result.err());
    }

    #[test]
    fn parse_nested_object() {
        // Main.C2.n - nested object with edge
        let input = "box at Main.C2.n";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_position_plus_offset() {
        // place + (x, y) offset
        let input = "box at C2.n + (0.35, 0.35)";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_test02() {
        let input = include_str!("../../pikchr/tests/test02.pikchr");
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse test02.pikchr: {:?}", result.err());
    }

    #[test]
    fn parse_one_dot_se() {
        // Test One.se as place - object with dot edge
        let input = "One.se";
        let result = PikchrParser::parse(Rule::place, input);
        println!("place('One.se'): {:?}", result);
        assert!(result.is_ok(), "Failed to parse as place: {:?}", result.err());

        // Test One.se as position
        let result = PikchrParser::parse(Rule::position, input);
        println!("position('One.se'): {:?}", result);
        assert!(result.is_ok(), "Failed to parse as position: {:?}", result.err());
    }

    #[test]
    fn parse_then_to_one_se() {
        // Test progressively to find where it breaks
        let tests = [
            "spline to One.se",
            "spline then to One.se",
            "spline -> to One.se",
            "spline left to One.se",
            "spline left 2cm to One.se",
            "spline -> left 2cm to One.se",
            "spline -> left 2cm then to One.se",
        ];
        for input in tests {
            let result = PikchrParser::parse(Rule::program, input);
            println!("{}: {}", input, if result.is_ok() { "OK" } else { "FAIL" });
            if result.is_err() {
                println!("  {:?}", result.err());
            }
        }
        // Final assertion
        let input = "spline -> left 2cm then to One.se";
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn parse_test03() {
        let input = include_str!("../../pikchr/tests/test03.pikchr");
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse test03.pikchr: {:?}", result.err());
    }

    #[test]
    fn parse_test10() {
        let input = include_str!("../../pikchr/tests/test10.pikchr");
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse test10.pikchr: {:?}", result.err());
    }

    #[test]
    fn parse_expr_file() {
        let input = include_str!("../../pikchr/tests/expr.pikchr");
        let result = PikchrParser::parse(Rule::program, input);
        assert!(result.is_ok(), "Failed to parse expr.pikchr: {:?}", result.err());
    }

    #[test]
    fn parse_all_pikchr_files() {
        // Files that are intentionally testing error handling (contain intentional syntax errors)
        let error_test_files = ["test60.pikchr", "test62.pikchr"];

        let test_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../pikchr/tests"));
        let mut pass = 0;
        let mut fail = 0;
        let mut expected_errors = 0;
        let mut failures = Vec::new();

        for entry in std::fs::read_dir(test_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "pikchr").unwrap_or(false) {
                let filename = path.file_name().unwrap().to_string_lossy();
                let source = std::fs::read_to_string(&path).unwrap();

                match PikchrParser::parse(Rule::program, &source) {
                    Ok(_) => pass += 1,
                    Err(e) => {
                        if error_test_files.contains(&filename.as_ref()) {
                            expected_errors += 1;
                        } else {
                            fail += 1;
                            failures.push((filename.to_string(), e.to_string()));
                        }
                    }
                }
            }
        }

        println!("\nParse results: {} passed, {} expected errors, {} unexpected failures", pass, expected_errors, fail);
        for (name, err) in &failures {
            println!("  FAIL: {} - {}", name, err.lines().next().unwrap_or(""));
        }
        assert!(failures.is_empty(), "{} files failed to parse unexpectedly", fail);
    }
}
