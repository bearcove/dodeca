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
    fn parse_expr_last_arrow() {
        // Test expr parsing of "last arrow"
        let input = "last arrow";
        let result = PikchrParser::parse(Rule::expr, input);
        assert!(result.is_ok(), "Failed to parse expr: {:?}", result.err());
    }

    #[test]
    fn parse_assert_stmt() {
        // Test assert statement
        let input = "assert( previous == last arrow )";
        let result = PikchrParser::parse(Rule::assert_stmt, input);
        assert!(result.is_ok(), "Failed to parse assert_stmt: {:?}", result.err());
    }

    #[test]
    fn parse_expr_just_last() {
        // Does "last" alone parse as expr?
        let input = "last";
        let result = PikchrParser::parse(Rule::expr, input);
        println!("Result for 'last': {:?}", result);
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
}
