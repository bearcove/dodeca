//! Parse pest pairs into AST nodes

use crate::ast::*;
use crate::{PikchrParser, Rule};
use pest::Parser;
use pest::iterators::Pair;

/// Parse pikchr source into AST
pub fn parse(source: &str) -> Result<Program, miette::Report> {
    let pairs = PikchrParser::parse(Rule::program, source)
        .map_err(|e| miette::miette!("Parse error: {}", e))?;

    let mut statements = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::statement_list {
                    statements = parse_statement_list(inner)?;
                }
            }
        }
    }

    Ok(Program { statements })
}

fn parse_statement_list(pair: Pair<Rule>) -> Result<Vec<Statement>, miette::Report> {
    let mut statements = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::statement {
            statements.push(parse_statement(inner)?);
        }
    }
    Ok(statements)
}

fn parse_statement(pair: Pair<Rule>) -> Result<Statement, miette::Report> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::labeled_statement => Ok(Statement::Labeled(parse_labeled_statement(inner)?)),
        Rule::direction => Ok(Statement::Direction(parse_direction(inner)?)),
        Rule::assignment => Ok(Statement::Assignment(parse_assignment(inner)?)),
        Rule::define => Ok(Statement::Define(parse_define(inner)?)),
        Rule::macro_call => Ok(Statement::MacroCall(parse_macro_call(inner)?)),
        Rule::assert_stmt => Ok(Statement::Assert(parse_assert(inner)?)),
        Rule::print_stmt => Ok(Statement::Print(parse_print(inner)?)),
        Rule::error_stmt => Ok(Statement::Error(parse_error_stmt(inner)?)),
        Rule::object_stmt => Ok(Statement::Object(parse_object_stmt(inner)?)),
        _ => Err(miette::miette!(
            "Unexpected rule in statement: {:?}",
            inner.as_rule()
        )),
    }
}

fn parse_direction(pair: Pair<Rule>) -> Result<Direction, miette::Report> {
    let s = pair.as_str().trim();
    match s {
        "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        "left" => Ok(Direction::Left),
        "right" => Ok(Direction::Right),
        _ => Err(miette::miette!("Invalid direction: {}", s)),
    }
}

fn parse_assignment(pair: Pair<Rule>) -> Result<Assignment, miette::Report> {
    let mut inner = pair.into_inner();
    let lvalue = parse_lvalue(inner.next().unwrap())?;
    let op = parse_assign_op(inner.next().unwrap())?;
    let rvalue = parse_rvalue(inner.next().unwrap())?;
    Ok(Assignment { lvalue, op, rvalue })
}

fn parse_lvalue(pair: Pair<Rule>) -> Result<LValue, miette::Report> {
    // Grammar: lvalue = { variable | "fill" | "color" | "thickness" }
    // If it's a literal like "fill", there may be no inner children
    let pair_str = pair.as_str();

    if let Some(inner) = pair.into_inner().next() {
        match inner.as_rule() {
            Rule::variable => Ok(LValue::Variable(parse_variable_name(inner)?)),
            _ => {
                let s = inner.as_str();
                match s {
                    "fill" => Ok(LValue::Fill),
                    "color" => Ok(LValue::Color),
                    "thickness" => Ok(LValue::Thickness),
                    _ => Err(miette::miette!("Invalid lvalue: {}", s)),
                }
            }
        }
    } else {
        // No inner children - check the string directly
        match pair_str.trim() {
            "fill" => Ok(LValue::Fill),
            "color" => Ok(LValue::Color),
            "thickness" => Ok(LValue::Thickness),
            s => Err(miette::miette!("Invalid lvalue with no children: {}", s)),
        }
    }
}

fn parse_variable_name(pair: Pair<Rule>) -> Result<String, miette::Report> {
    // cref: pik_value (pikchr.c) - variables can be "$foo" or "foo"
    // The "$" prefix is part of the variable name - "$margin" is different from "margin"
    // This is important because system variables like "margin", "thickness" etc. should not
    // be confused with user-defined "$margin", "$thickness" etc.
    let raw = pair.as_str();
    if raw.starts_with('$') {
        // Variable has $ prefix - include it in the name
        Ok(raw.to_string())
    } else {
        // No $ prefix - just the bare identifier
        let mut name = String::new();
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::IDENT | Rule::PLACENAME => name = inner.as_str().to_string(),
                _ => {}
            }
        }
        Ok(name)
    }
}

fn parse_assign_op(pair: Pair<Rule>) -> Result<AssignOp, miette::Report> {
    match pair.as_str() {
        "=" => Ok(AssignOp::Assign),
        "+=" => Ok(AssignOp::AddAssign),
        "-=" => Ok(AssignOp::SubAssign),
        "*=" => Ok(AssignOp::MulAssign),
        "/=" => Ok(AssignOp::DivAssign),
        s => Err(miette::miette!("Invalid assign op: {}", s)),
    }
}

fn parse_rvalue(pair: Pair<Rule>) -> Result<RValue, miette::Report> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::expr => Ok(RValue::Expr(parse_expr(inner)?)),
        Rule::PLACENAME => Ok(RValue::PlaceName(inner.as_str().to_string())),
        Rule::HEX_COLOR => Ok(RValue::PlaceName(inner.as_str().to_string())), // Pass hex color as-is
        _ => Err(miette::miette!("Invalid rvalue: {:?}", inner.as_rule())),
    }
}

fn parse_define(pair: Pair<Rule>) -> Result<Define, miette::Report> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let body = inner.next().unwrap().as_str().to_string();
    Ok(Define { name, body })
}

fn parse_macro_call(pair: Pair<Rule>) -> Result<MacroCall, miette::Report> {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let args = if let Some(args_pair) = inner.next() {
        parse_macro_args(args_pair)?
    } else {
        Vec::new()
    };
    Ok(MacroCall { name, args })
}

fn parse_macro_args(pair: Pair<Rule>) -> Result<Vec<MacroArg>, miette::Report> {
    let mut args = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::macro_arg {
            args.push(parse_macro_arg(inner)?);
        }
    }
    Ok(args)
}

fn parse_macro_arg(pair: Pair<Rule>) -> Result<MacroArg, miette::Report> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::STRING => Ok(MacroArg::String(parse_string(inner)?)),
        Rule::expr => Ok(MacroArg::Expr(parse_expr(inner)?)),
        Rule::IDENT => Ok(MacroArg::Ident(inner.as_str().to_string())),
        _ => Err(miette::miette!("Invalid macro arg: {:?}", inner.as_rule())),
    }
}

fn parse_assert(pair: Pair<Rule>) -> Result<Assert, miette::Report> {
    let mut inner = pair.into_inner().peekable();
    // Grammar: "assert" ~ "(" ~ (expr ~ "==" ~ expr | position ~ "==" ~ position) ~ ")"
    // Keywords/literals like "assert", "(", "==", ")" are not captured as children

    let first = inner
        .next()
        .ok_or_else(|| miette::miette!("Empty assert statement"))?;

    let condition = if first.as_rule() == Rule::expr {
        let left = parse_expr(first)?;
        // "==" is a literal, not captured - next should be the second expr
        let right_pair = inner
            .next()
            .ok_or_else(|| miette::miette!("Missing right side of assert"))?;
        let right = parse_expr(right_pair)?;
        AssertCondition::ExprEqual(left, right)
    } else if first.as_rule() == Rule::position {
        let left = parse_position(first)?;
        // "==" is a literal, not captured - next should be the second position
        let right_pair = inner
            .next()
            .ok_or_else(|| miette::miette!("Missing right side of assert"))?;
        let right = parse_position(right_pair)?;
        AssertCondition::PositionEqual(left, right)
    } else {
        return Err(miette::miette!(
            "Invalid assert condition: {:?}",
            first.as_rule()
        ));
    };
    Ok(Assert { condition })
}

fn parse_print(pair: Pair<Rule>) -> Result<Print, miette::Report> {
    let mut args = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::print_args {
            for arg_pair in inner.into_inner() {
                if arg_pair.as_rule() == Rule::print_arg {
                    let arg_inner = arg_pair.into_inner().next().unwrap();
                    let arg = match arg_inner.as_rule() {
                        Rule::STRING => PrintArg::String(parse_string(arg_inner)?),
                        Rule::expr => PrintArg::Expr(parse_expr(arg_inner)?),
                        Rule::PLACENAME => PrintArg::PlaceName(arg_inner.as_str().to_string()),
                        _ => continue,
                    };
                    args.push(arg);
                }
            }
        }
    }
    Ok(Print { args })
}

fn parse_error_stmt(pair: Pair<Rule>) -> Result<ErrorStmt, miette::Report> {
    let inner = pair.into_inner().next().unwrap();
    let message = parse_string(inner)?;
    Ok(ErrorStmt { message })
}

fn parse_labeled_statement(pair: Pair<Rule>) -> Result<LabeledStatement, miette::Report> {
    let mut inner = pair.into_inner();
    let label = inner.next().unwrap().as_str().to_string();
    let content_pair = inner.next().unwrap();
    let content = match content_pair.as_rule() {
        Rule::position => LabeledContent::Position(parse_position(content_pair)?),
        Rule::object_stmt => LabeledContent::Object(parse_object_stmt(content_pair)?),
        _ => {
            return Err(miette::miette!(
                "Invalid labeled content: {:?}",
                content_pair.as_rule()
            ));
        }
    };
    Ok(LabeledStatement { label, content })
}

fn parse_object_stmt(pair: Pair<Rule>) -> Result<ObjectStatement, miette::Report> {
    let mut inner = pair.into_inner();
    let basetype = parse_basetype(inner.next().unwrap())?;
    let mut attributes = Vec::new();
    for attr_list in inner {
        if attr_list.as_rule() == Rule::attribute_list {
            for attr in attr_list.into_inner() {
                if attr.as_rule() == Rule::attribute {
                    attributes.push(parse_attribute(attr)?);
                }
            }
        }
    }
    Ok(ObjectStatement {
        basetype,
        attributes,
    })
}

fn parse_basetype(pair: Pair<Rule>) -> Result<BaseType, miette::Report> {
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();
    match first.as_rule() {
        Rule::CLASSNAME => Ok(BaseType::Class(parse_classname(first)?)),
        Rule::STRING => {
            let text = parse_string(first)?;
            // Check for optional textposition (e.g., rjust, ljust, above, below)
            let textpos = inner
                .next()
                .filter(|p| p.as_rule() == Rule::textposition)
                .map(parse_textposition)
                .transpose()?;
            Ok(BaseType::Text(StringLit { value: text }, textpos))
        }
        Rule::sublist => {
            let statements = parse_statement_list(first.into_inner().next().unwrap())?;
            Ok(BaseType::Sublist(statements))
        }
        _ => Err(miette::miette!("Invalid basetype: {:?}", first.as_rule())),
    }
}

fn parse_classname(pair: Pair<Rule>) -> Result<ClassName, miette::Report> {
    match pair.as_str() {
        "arc" => Ok(ClassName::Arc),
        "arrow" => Ok(ClassName::Arrow),
        "box" => Ok(ClassName::Box),
        "circle" => Ok(ClassName::Circle),
        "cylinder" => Ok(ClassName::Cylinder),
        "diamond" => Ok(ClassName::Diamond),
        "dot" => Ok(ClassName::Dot),
        "ellipse" => Ok(ClassName::Ellipse),
        "file" => Ok(ClassName::File),
        "line" => Ok(ClassName::Line),
        "move" => Ok(ClassName::Move),
        "oval" => Ok(ClassName::Oval),
        "spline" => Ok(ClassName::Spline),
        "text" => Ok(ClassName::Text),
        s => Err(miette::miette!("Invalid classname: {}", s)),
    }
}

fn parse_attribute(pair: Pair<Rule>) -> Result<Attribute, miette::Report> {
    let pair_str = pair.as_str().to_string();

    // Debug: log what attribute we're parsing
    tracing::debug!("parse_attribute: pair_str={:?}", pair_str);

    let mut inner = pair.into_inner().peekable();

    // If no inner pairs, check the raw string
    if inner.peek().is_none() {
        return match pair_str.trim() {
            "close" => Ok(Attribute::Close),
            "chop" => Ok(Attribute::Chop),
            "fit" => Ok(Attribute::Fit),
            "then" => Ok(Attribute::Then(None)),
            "same" => Ok(Attribute::Same(None)),
            s => Err(miette::miette!("Unexpected empty attribute: {}", s)),
        };
    }

    // Peek at what kind of attribute this is
    let first = inner.peek().unwrap();

    match first.as_rule() {
        Rule::numproperty => {
            let prop = parse_numproperty(inner.next().unwrap())?;
            let relexpr = parse_relexpr(inner.next().unwrap())?;
            Ok(Attribute::NumProperty(prop, relexpr))
        }
        Rule::dashproperty => {
            let prop = parse_dashproperty(inner.next().unwrap())?;
            let expr = inner.next().map(|p| parse_expr(p)).transpose()?;
            Ok(Attribute::DashProperty(prop, expr))
        }
        Rule::colorproperty => {
            let prop = parse_colorproperty(inner.next().unwrap())?;
            let rvalue = parse_rvalue(inner.next().unwrap())?;
            Ok(Attribute::ColorProperty(prop, rvalue))
        }
        Rule::boolproperty => {
            let prop = parse_boolproperty(inner.next().unwrap())?;
            Ok(Attribute::BoolProperty(prop))
        }
        Rule::STRING => {
            let s = parse_string(inner.next().unwrap())?;
            let textpos = inner.next().map(|p| parse_textposition(p)).transpose()?;
            Ok(Attribute::StringAttr(StringLit { value: s }, textpos))
        }
        Rule::relexpr => {
            let relexpr = parse_relexpr(inner.next().unwrap())?;
            // Check if this is actually "relexpr heading expr"
            if inner
                .peek()
                .map(|p| p.as_str() == "heading")
                .unwrap_or(false)
            {
                inner.next(); // skip "heading"
                let heading_expr = parse_expr(inner.next().unwrap())?;
                Ok(Attribute::Heading(Some(relexpr), heading_expr))
            } else {
                Ok(Attribute::BareExpr(relexpr))
            }
        }
        Rule::optrelexpr => {
            // optrelexpr can be empty - Grammar: optrelexpr = { relexpr? }
            // This might appear in: "go"? ~ optrelexpr ~ "heading" ~ expr
            // Or in: "then" ~ optrelexpr ~ "heading" ~ expr
            let is_then = pair_str.trim_start().starts_with("then");
            let opt = inner.next().unwrap();
            let relexpr = opt
                .into_inner()
                .next()
                .map(|p| parse_relexpr(p))
                .transpose()?;

            // Check what follows
            // Note: "heading" is a literal consumed by pest, not a token
            // So we check pair_str to detect it, and the next token is the expr
            let has_heading = pair_str.contains("heading");
            if has_heading {
                // The next token after optrelexpr is the heading expr (not "heading" itself)
                let heading_expr = parse_expr(inner.next().unwrap())?;
                if is_then {
                    // "then [optrelexpr] heading expr" -> ThenClause::Heading
                    Ok(Attribute::Then(Some(ThenClause::Heading(
                        relexpr,
                        heading_expr,
                    ))))
                } else {
                    // "[go] [optrelexpr] heading expr" -> Attribute::Heading
                    Ok(Attribute::Heading(relexpr, heading_expr))
                }
            } else if inner
                .peek()
                .map(|p| p.as_rule() == Rule::EDGEPT)
                .unwrap_or(false)
            {
                // optrelexpr EDGEPT - this is a then clause variant
                let ep = parse_edgepoint(inner.next().unwrap())?;
                Ok(Attribute::Then(Some(ThenClause::EdgePoint(relexpr, ep))))
            } else if let Some(re) = relexpr {
                // Just an optrelexpr - treat as bare expression
                Ok(Attribute::BareExpr(re))
            } else {
                // Empty optrelexpr without anything following
                // This can happen in some edge cases - just ignore it as "then with no clause"
                Ok(Attribute::Then(None))
            }
        }
        Rule::direction => {
            // Check if this is a "then direction ..." clause
            if pair_str.trim_start().starts_with("then") {
                // This is a then clause with direction
                let dir = parse_direction(inner.next().unwrap())?;
                // Check what follows
                if let Some(next) = inner.next() {
                    match next.as_rule() {
                        Rule::optrelexpr => {
                            let relexpr = next
                                .into_inner()
                                .next()
                                .map(|p| parse_relexpr(p))
                                .transpose()?;
                            Ok(Attribute::Then(Some(ThenClause::DirectionMove(
                                dir, relexpr,
                            ))))
                        }
                        Rule::position => {
                            // direction even with position or direction until even with position
                            let pos = parse_position(next)?;
                            if pair_str.contains("until") {
                                Ok(Attribute::Then(Some(ThenClause::DirectionUntilEven(
                                    dir, pos,
                                ))))
                            } else {
                                Ok(Attribute::Then(Some(ThenClause::DirectionEven(dir, pos))))
                            }
                        }
                        _ => Ok(Attribute::Then(Some(ThenClause::DirectionMove(dir, None)))),
                    }
                } else {
                    Ok(Attribute::Then(Some(ThenClause::DirectionMove(dir, None))))
                }
            } else {
                // Regular direction with optional distance/position variants
                parse_direction_attribute(&mut inner, &pair_str)
            }
        }
        Rule::position => {
            let pos = parse_position(inner.next().unwrap())?;
            // Check if this is "from", "to", "at", or "then to" position based on the original string
            // (the keyword is a literal and not captured as a child)
            let trimmed = pair_str.trim_start();
            if trimmed.starts_with("from") {
                Ok(Attribute::From(pos))
            } else if trimmed.starts_with("at") {
                Ok(Attribute::At(pos))
            } else if trimmed.starts_with("then") {
                // "then to position" - new grammar rule
                Ok(Attribute::Then(Some(ThenClause::To(pos))))
            } else {
                Ok(Attribute::To(pos))
            }
        }
        Rule::withclause => {
            let clause = parse_withclause(inner.next().unwrap())?;
            Ok(Attribute::With(clause))
        }
        Rule::object => {
            // Check if this is actually a "same as object" attribute
            // (keywords "same" and "as" are not captured as children in pest)
            if pair_str.trim_start().starts_with("same") {
                let obj = parse_object(inner.next().unwrap())?;
                return Ok(Attribute::Same(Some(obj)));
            }
            let obj = parse_object(inner.next().unwrap())?;
            Ok(Attribute::Behind(obj))
        }
        _ => {
            // Check for keyword-based attributes by string matching
            let s = first.as_str();
            match s {
                "go" => {
                    inner.next(); // skip "go"
                    // After "go", check what follows
                    if let Some(next) = inner.peek() {
                        if next.as_rule() == Rule::direction {
                            parse_direction_attribute(&mut inner, &pair_str)
                        } else if next.as_rule() == Rule::optrelexpr {
                            // go optrelexpr heading expr
                            let opt = inner.next().unwrap();
                            let relexpr = opt
                                .into_inner()
                                .next()
                                .map(|p| parse_relexpr(p))
                                .transpose()?;
                            inner.next(); // skip "heading"
                            let heading_expr = parse_expr(inner.next().unwrap())?;
                            Ok(Attribute::Heading(relexpr, heading_expr))
                        } else {
                            Err(miette::miette!(
                                "Unexpected after 'go': {:?}",
                                next.as_rule()
                            ))
                        }
                    } else {
                        Err(miette::miette!("Nothing after 'go'"))
                    }
                }
                "close" => Ok(Attribute::Close),
                "chop" => Ok(Attribute::Chop),
                "from" => {
                    inner.next(); // skip "from"
                    let pos = parse_position(inner.next().unwrap())?;
                    Ok(Attribute::From(pos))
                }
                "to" => {
                    inner.next(); // skip "to"
                    let pos = parse_position(inner.next().unwrap())?;
                    Ok(Attribute::To(pos))
                }
                "then" => {
                    inner.next(); // skip "then"
                    parse_then_clause(&mut inner, &pair_str)
                }
                "at" => {
                    inner.next(); // skip "at"
                    let pos = parse_position(inner.next().unwrap())?;
                    Ok(Attribute::At(pos))
                }
                "with" => {
                    inner.next(); // skip "with"
                    let clause = parse_withclause(inner.next().unwrap())?;
                    Ok(Attribute::With(clause))
                }
                "same" => {
                    inner.next(); // skip "same"
                    // Skip "as" if present
                    if inner.peek().map(|p| p.as_str() == "as").unwrap_or(false) {
                        inner.next();
                    }
                    let object = inner.next().map(|p| parse_object(p)).transpose()?;
                    Ok(Attribute::Same(object))
                }
                "fit" => Ok(Attribute::Fit),
                "behind" => {
                    inner.next(); // skip "behind"
                    let obj = parse_object(inner.next().unwrap())?;
                    Ok(Attribute::Behind(obj))
                }
                _ => Err(miette::miette!(
                    "Unexpected attribute: {} (rule: {:?})",
                    s,
                    first.as_rule()
                )),
            }
        }
    }
}

fn parse_direction_attribute<'a, I>(
    inner: &mut std::iter::Peekable<I>,
    pair_str: &str,
) -> Result<Attribute, miette::Report>
where
    I: Iterator<Item = Pair<'a, Rule>>,
{
    // Parse direction
    let dir = parse_direction(inner.next().unwrap())?;

    // Check what follows
    if inner.peek().is_none() {
        return Ok(Attribute::DirectionMove(None, dir, None));
    }

    // Debug: log what tokens we have
    if let Some(peek) = inner.peek() {
        tracing::debug!(
            "parse_direction_attribute after direction: next rule={:?}, str={:?}, pair_str={:?}",
            peek.as_rule(),
            peek.as_str(),
            pair_str
        );
    }

    // Check the next rule - in pest, "until", "even", "with" are literals that get
    // consumed but don't produce tokens. So we need to check pair_str for context.
    if let Some(next) = inner.peek() {
        match next.as_rule() {
            Rule::position => {
                // This is "direction [until] even with position"
                // The keywords are consumed by pest as literals
                let pos = parse_position(inner.next().unwrap())?;
                if pair_str.contains("until") {
                    tracing::debug!("Parsed DirectionUntilEven: {:?}", dir);
                    Ok(Attribute::DirectionUntilEven(None, dir, pos))
                } else if pair_str.contains("even") {
                    tracing::debug!("Parsed DirectionEven: {:?}", dir);
                    Ok(Attribute::DirectionEven(None, dir, pos))
                } else {
                    // Just "direction position" - shouldn't happen in valid grammar
                    Ok(Attribute::DirectionMove(None, dir, None))
                }
            }
            Rule::optrelexpr => {
                // direction optrelexpr
                let relexpr = inner
                    .next()
                    .unwrap()
                    .into_inner()
                    .next()
                    .map(|r| parse_relexpr(r))
                    .transpose()?;
                Ok(Attribute::DirectionMove(None, dir, relexpr))
            }
            _ => {
                // Unknown pattern - just return direction move with no distance
                Ok(Attribute::DirectionMove(None, dir, None))
            }
        }
    } else {
        Ok(Attribute::DirectionMove(None, dir, None))
    }
}

fn parse_then_clause<'a, I>(
    inner: &mut std::iter::Peekable<I>,
    pair_str: &str,
) -> Result<Attribute, miette::Report>
where
    I: Iterator<Item = Pair<'a, Rule>>,
{
    // "then" has already been consumed
    // What follows could be:
    // - "to" position
    // - direction "until" "even" "with"? position
    // - direction "even" "with"? position
    // - direction optrelexpr
    // - optrelexpr "heading" expr
    // - optrelexpr EDGEPT
    // - nothing (bare then)

    if inner.peek().is_none() {
        return Ok(Attribute::Then(None));
    }

    let next = inner.peek().unwrap();
    let next_str = next.as_str();

    if next_str == "to" {
        inner.next(); // skip "to"
        let pos = parse_position(inner.next().unwrap())?;
        Ok(Attribute::Then(Some(ThenClause::To(pos))))
    } else if next.as_rule() == Rule::direction {
        let dir = parse_direction(inner.next().unwrap())?;

        // Check what follows the direction
        // Note: "until", "even", "with" are literals consumed by pest but not returned as tokens
        // So we check the rule type and use pair_str for context
        if let Some(after) = inner.peek() {
            match after.as_rule() {
                Rule::position => {
                    // This is "then direction [until] even with position"
                    let pos = parse_position(inner.next().unwrap())?;
                    if pair_str.contains("until") {
                        tracing::debug!("parse_then_clause: DirectionUntilEven {:?}", dir);
                        Ok(Attribute::Then(Some(ThenClause::DirectionUntilEven(
                            dir, pos,
                        ))))
                    } else if pair_str.contains("even") {
                        tracing::debug!("parse_then_clause: DirectionEven {:?}", dir);
                        Ok(Attribute::Then(Some(ThenClause::DirectionEven(dir, pos))))
                    } else {
                        // Shouldn't happen - direction followed by position without even
                        Ok(Attribute::Then(Some(ThenClause::DirectionMove(dir, None))))
                    }
                }
                Rule::optrelexpr => {
                    let relexpr = inner
                        .next()
                        .unwrap()
                        .into_inner()
                        .next()
                        .map(|p| parse_relexpr(p))
                        .transpose()?;
                    Ok(Attribute::Then(Some(ThenClause::DirectionMove(
                        dir, relexpr,
                    ))))
                }
                _ => Ok(Attribute::Then(Some(ThenClause::DirectionMove(dir, None)))),
            }
        } else {
            Ok(Attribute::Then(Some(ThenClause::DirectionMove(dir, None))))
        }
    } else if next.as_rule() == Rule::optrelexpr {
        let opt = inner.next().unwrap();
        let relexpr = opt
            .into_inner()
            .next()
            .map(|p| parse_relexpr(p))
            .transpose()?;

        // Check for "heading" or EDGEPT
        if let Some(after) = inner.peek() {
            if after.as_str() == "heading" {
                inner.next(); // skip "heading"
                let heading_expr = parse_expr(inner.next().unwrap())?;
                Ok(Attribute::Then(Some(ThenClause::Heading(
                    relexpr,
                    heading_expr,
                ))))
            } else if after.as_rule() == Rule::EDGEPT {
                let ep = parse_edgepoint(inner.next().unwrap())?;
                Ok(Attribute::Then(Some(ThenClause::EdgePoint(relexpr, ep))))
            } else {
                // Just then with no clause
                Ok(Attribute::Then(None))
            }
        } else {
            Ok(Attribute::Then(None))
        }
    } else {
        Ok(Attribute::Then(None))
    }
}

fn parse_numproperty(pair: Pair<Rule>) -> Result<NumProperty, miette::Report> {
    match pair.as_str() {
        "height" | "ht" => Ok(NumProperty::Height),
        "width" | "wid" => Ok(NumProperty::Width),
        "radius" | "rad" => Ok(NumProperty::Radius),
        "diameter" => Ok(NumProperty::Diameter),
        "thickness" => Ok(NumProperty::Thickness),
        s => Err(miette::miette!("Invalid numproperty: {}", s)),
    }
}

fn parse_dashproperty(pair: Pair<Rule>) -> Result<DashProperty, miette::Report> {
    match pair.as_str() {
        "dotted" => Ok(DashProperty::Dotted),
        "dashed" => Ok(DashProperty::Dashed),
        s => Err(miette::miette!("Invalid dashproperty: {}", s)),
    }
}

fn parse_colorproperty(pair: Pair<Rule>) -> Result<ColorProperty, miette::Report> {
    match pair.as_str() {
        "fill" => Ok(ColorProperty::Fill),
        "color" => Ok(ColorProperty::Color),
        s => Err(miette::miette!("Invalid colorproperty: {}", s)),
    }
}

fn parse_boolproperty(pair: Pair<Rule>) -> Result<BoolProperty, miette::Report> {
    match pair.as_str() {
        "cw" => Ok(BoolProperty::Clockwise),
        "ccw" => Ok(BoolProperty::CounterClockwise),
        "invis" | "invisible" => Ok(BoolProperty::Invisible),
        "thick" => Ok(BoolProperty::Thick),
        "thin" => Ok(BoolProperty::Thin),
        "solid" => Ok(BoolProperty::Solid),
        "<->" | "&leftrightarrow;" | "↔" => Ok(BoolProperty::ArrowBoth),
        "->" | "&rarr;" | "&rightarrow;" | "→" => Ok(BoolProperty::ArrowRight),
        "<-" | "&larr;" | "&leftarrow;" | "←" => Ok(BoolProperty::ArrowLeft),
        s => Err(miette::miette!("Invalid boolproperty: {}", s)),
    }
}

fn parse_withclause(pair: Pair<Rule>) -> Result<WithClause, miette::Report> {
    let mut inner = pair.into_inner().peekable();

    // First child should be dot_edge or EDGEPT
    let edge_pair = inner
        .next()
        .ok_or_else(|| miette::miette!("Empty withclause"))?;

    let edge = match edge_pair.as_rule() {
        Rule::dot_edge => {
            // dot_edge = { "." ~ EDGEPT }
            let edgept_str = edge_pair.as_str();
            // Extract the edge point from the string (after the dot)
            let ep_str = edgept_str.trim_start_matches('.');
            let ep = parse_edgepoint_str(ep_str)?;
            WithEdge::DotEdge(ep)
        }
        Rule::EDGEPT => {
            let ep = parse_edgepoint(edge_pair)?;
            WithEdge::EdgePoint(ep)
        }
        Rule::position => {
            // The edge was not parsed, position came first - this means the edge was parsed
            // differently. Look at the original string.
            let pos = parse_position(edge_pair)?;
            // Default to center if we can't find an edge
            return Ok(WithClause {
                edge: WithEdge::EdgePoint(EdgePoint::Center),
                position: pos,
            });
        }
        _ => {
            return Err(miette::miette!(
                "Invalid withclause edge: {:?}",
                edge_pair.as_rule()
            ));
        }
    };

    // "at" is a keyword literal - not captured as a child
    // Next should be position
    let position = if let Some(pos_pair) = inner.next() {
        parse_position(pos_pair)?
    } else {
        return Err(miette::miette!("Missing position in withclause"));
    };

    Ok(WithClause { edge, position })
}

fn parse_edgepoint_str(s: &str) -> Result<EdgePoint, miette::Report> {
    match s.trim() {
        "north" | "n" => Ok(EdgePoint::North),
        "south" | "s" => Ok(EdgePoint::South),
        "east" | "e" => Ok(EdgePoint::East),
        "west" | "w" => Ok(EdgePoint::West),
        "start" => Ok(EdgePoint::Start),
        "end" => Ok(EdgePoint::End),
        "center" | "c" => Ok(EdgePoint::Center),
        "bottom" | "bot" => Ok(EdgePoint::Bottom),
        "top" | "t" => Ok(EdgePoint::Top),
        "left" => Ok(EdgePoint::Left),
        "right" => Ok(EdgePoint::Right),
        "ne" => Ok(EdgePoint::NorthEast),
        "nw" => Ok(EdgePoint::NorthWest),
        "se" => Ok(EdgePoint::SouthEast),
        "sw" => Ok(EdgePoint::SouthWest),
        _ => Err(miette::miette!("Invalid edgepoint string: {}", s)),
    }
}

fn parse_textposition(pair: Pair<Rule>) -> Result<TextPosition, miette::Report> {
    let mut attrs = Vec::new();
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::textattr {
            attrs.push(parse_textattr(inner)?);
        }
    }
    Ok(TextPosition { attrs })
}

fn parse_textattr(pair: Pair<Rule>) -> Result<TextAttr, miette::Report> {
    match pair.as_str() {
        "above" => Ok(TextAttr::Above),
        "below" => Ok(TextAttr::Below),
        "center" => Ok(TextAttr::Center),
        "ljust" => Ok(TextAttr::LJust),
        "rjust" => Ok(TextAttr::RJust),
        "bold" => Ok(TextAttr::Bold),
        "italic" => Ok(TextAttr::Italic),
        "mono" | "monospace" => Ok(TextAttr::Mono),
        "big" => Ok(TextAttr::Big),
        "small" => Ok(TextAttr::Small),
        "aligned" => Ok(TextAttr::Aligned),
        s => Err(miette::miette!("Invalid textattr: {}", s)),
    }
}

fn parse_relexpr(pair: Pair<Rule>) -> Result<RelExpr, miette::Report> {
    let mut inner = pair.into_inner();
    let expr = parse_expr(inner.next().unwrap())?;
    // Check if there's a percent rule following the expression
    let is_percent = inner
        .next()
        .map(|p| p.as_rule() == Rule::percent)
        .unwrap_or(false);
    Ok(RelExpr { expr, is_percent })
}

fn parse_expr(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    // expr = term ~ (add_op ~ term)*
    let mut inner = pair.into_inner();
    let mut result = parse_term(inner.next().unwrap())?;

    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() != Rule::add_op {
            continue;
        }
        let op = match op_pair.as_str() {
            "+" => BinaryOp::Add,
            "-" => BinaryOp::Sub,
            _ => continue,
        };
        let rhs = parse_term(inner.next().unwrap())?;
        result = Expr::BinaryOp(Box::new(result), op, Box::new(rhs));
    }

    Ok(result)
}

fn parse_term(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    // term = prefix? ~ primary ~ (mul_op ~ prefix? ~ primary)*
    let mut inner = pair.into_inner().peekable();

    // Handle prefix
    let mut prefix: Option<UnaryOp> = None;
    if inner.peek().map(|p| p.as_rule()) == Some(Rule::prefix) {
        let p = inner.next().unwrap();
        prefix = Some(match p.as_str() {
            "-" => UnaryOp::Neg,
            "+" => UnaryOp::Pos,
            _ => return Err(miette::miette!("Invalid prefix: {}", p.as_str())),
        });
    }

    // Parse primary
    let primary_pair = inner.next().unwrap();
    let mut result = parse_primary(primary_pair)?;

    // Apply prefix
    if let Some(op) = prefix {
        result = Expr::UnaryOp(op, Box::new(result));
    }

    // Handle mul/div operations
    while let Some(op_pair) = inner.next() {
        if op_pair.as_rule() != Rule::mul_op {
            continue;
        }
        let op = match op_pair.as_str() {
            "*" => BinaryOp::Mul,
            "/" => BinaryOp::Div,
            _ => continue,
        };

        // Handle possible prefix on next operand
        let mut rhs_prefix: Option<UnaryOp> = None;
        if inner.peek().map(|p| p.as_rule()) == Some(Rule::prefix) {
            let p = inner.next().unwrap();
            rhs_prefix = Some(match p.as_str() {
                "-" => UnaryOp::Neg,
                "+" => UnaryOp::Pos,
                _ => return Err(miette::miette!("Invalid prefix")),
            });
        }

        let rhs_primary = inner.next().unwrap();
        let mut rhs = parse_primary(rhs_primary)?;

        if let Some(op) = rhs_prefix {
            rhs = Expr::UnaryOp(op, Box::new(rhs));
        }

        result = Expr::BinaryOp(Box::new(result), op, Box::new(rhs));
    }

    Ok(result)
}

fn parse_primary(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    let mut inner = pair.into_inner().peekable();
    let first = inner.next().unwrap();

    match first.as_rule() {
        Rule::expr => {
            // Parenthesized expression: (expr) or (fill|color|thickness)
            Ok(Expr::ParenExpr(Box::new(parse_expr(first)?)))
        }
        Rule::func_call => parse_func_call(first),
        Rule::dist_call => parse_dist_call(first),
        Rule::NUMBER => parse_number(first),
        Rule::variable => Ok(Expr::Variable(parse_variable_name(first)?)),
        Rule::NTH => {
            // Grammar: NTH ~ "vertex" ~ "of" ~ object ~ dot_xy
            // "vertex" and "of" are literals, not captured
            let nth = parse_nth_from_str(first.as_str())?;
            // Next should be object, then dot_xy
            let obj_pair = inner
                .next()
                .ok_or_else(|| miette::miette!("Missing object in vertex expression"))?;
            let obj = parse_object(obj_pair)?;
            let coord_pair = inner
                .next()
                .ok_or_else(|| miette::miette!("Missing coordinate in vertex expression"))?;
            let coord = parse_coord(coord_pair)?;
            Ok(Expr::VertexCoord(nth, obj, coord))
        }
        Rule::object => {
            let obj = parse_object(first)?;

            // Check what follows: dot_edge + dot_xy, dot_xy, or dot_prop
            if let Some(next) = inner.next() {
                match next.as_rule() {
                    Rule::dot_edge => {
                        // object.edge.xy
                        let ep = parse_edgepoint(next.into_inner().next().unwrap())?;
                        if let Some(xy_pair) = inner.next() {
                            let coord = parse_coord(xy_pair)?;
                            Ok(Expr::ObjectEdgeCoord(obj, ep, coord))
                        } else {
                            // object.edge without .xy - this is a place, not an expr
                            Err(miette::miette!("object.edge is a place, not an expression"))
                        }
                    }
                    Rule::dot_xy => {
                        // object.x or object.y
                        let coord = parse_coord(next)?;
                        Ok(Expr::ObjectCoord(obj, coord))
                    }
                    Rule::dot_prop => {
                        // object.width, object.height, etc.
                        let prop_pair = next.into_inner().next().unwrap();
                        let prop = parse_numproperty(prop_pair)?;
                        Ok(Expr::ObjectProp(obj, prop))
                    }
                    _ => Err(miette::miette!(
                        "Unexpected after object in primary: {:?}",
                        next.as_rule()
                    )),
                }
            } else {
                // Bare object - this should be a place, not an expr
                Err(miette::miette!("Bare object is a place, not an expression"))
            }
        }
        _ => {
            // Check for builtin vars: (fill), (color), (thickness)
            let s = first.as_str();
            match s {
                "fill" => Ok(Expr::BuiltinVar(BuiltinVar::Fill)),
                "color" => Ok(Expr::BuiltinVar(BuiltinVar::Color)),
                "thickness" => Ok(Expr::BuiltinVar(BuiltinVar::Thickness)),
                _ => Err(miette::miette!(
                    "Unexpected primary: {} (rule: {:?})",
                    s,
                    first.as_rule()
                )),
            }
        }
    }
}

fn parse_coord(pair: Pair<Rule>) -> Result<Coord, miette::Report> {
    // dot_xy = { "." ~ ("x" | "y") } - literals don't create children
    // So we need to look at the string content
    let s = pair.as_str();
    if s.contains('x') {
        Ok(Coord::X)
    } else if s.contains('y') {
        Ok(Coord::Y)
    } else {
        Err(miette::miette!("Invalid coord: {}", s))
    }
}

fn parse_nth_from_str(s: &str) -> Result<Nth, miette::Report> {
    // Parse ordinal like "1st", "2nd", "3rd", etc.
    let num: u32 = s
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .map_err(|_| miette::miette!("Invalid ordinal: {}", s))?;
    Ok(Nth::Ordinal(num, false, None))
}

fn parse_func_call(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    let mut inner = pair.into_inner();
    let func_pair = inner.next().unwrap();
    let func = match func_pair.as_str() {
        "abs" => Function::Abs,
        "cos" => Function::Cos,
        "sin" => Function::Sin,
        "int" => Function::Int,
        "sqrt" => Function::Sqrt,
        "max" => Function::Max,
        "min" => Function::Min,
        s => return Err(miette::miette!("Unknown function: {}", s)),
    };
    let mut args = Vec::new();
    for arg in inner {
        if arg.as_rule() == Rule::expr {
            args.push(parse_expr(arg)?);
        }
    }
    Ok(Expr::FuncCall(FuncCall { func, args }))
}

fn parse_dist_call(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    let mut inner = pair.into_inner();
    let pos1 = parse_position(inner.next().unwrap())?;
    let pos2 = parse_position(inner.next().unwrap())?;
    Ok(Expr::DistCall(Box::new(pos1), Box::new(pos2)))
}

fn parse_number(pair: Pair<Rule>) -> Result<Expr, miette::Report> {
    let raw = pair.as_str();

    // Hex literal (kept as-is, like C)
    if raw.starts_with("0x") || raw.starts_with("0X") {
        let n = u64::from_str_radix(&raw[2..], 16)
            .map_err(|e| miette::miette!("Invalid hex number: {}", e))?;
        return Ok(Expr::Number(n as f64));
    }

    // Detect 2-letter unit suffix (in/cm/mm/pt/px/pc)
    let (number_part, unit_suffix) = if raw.len() >= 2 {
        let (head, tail) = raw.split_at(raw.len() - 2);
        match tail {
            "in" | "cm" | "mm" | "pt" | "px" | "pc" => (head, Some(tail)),
            _ => (raw, None),
        }
    } else {
        (raw, None)
    };

    let mut n: f64 = number_part
        .parse()
        .map_err(|e| miette::miette!("Invalid number: {}", e))?;

    // Convert to inches to mirror pikchr.c:pik_atof
    if let Some(unit) = unit_suffix {
        n = match unit {
            "in" => n,
            "cm" => n / 2.54,
            "mm" => n / 25.4,
            "px" => n / 96.0,
            "pt" => n / 72.0,
            "pc" => n / 6.0,
            _ => n, // unreachable due to match above
        };
    }

    Ok(Expr::Number(n))
}

fn parse_position(pair: Pair<Rule>) -> Result<Position, miette::Report> {
    let pair_str = pair.as_str().to_string();
    let mut inner = pair.into_inner();

    let child = inner
        .next()
        .ok_or_else(|| miette::miette!("Empty position: {}", pair_str))?;

    match child.as_rule() {
        Rule::pos_tuple => {
            // "(" ~ position ~ "," ~ position ~ ")"
            let mut kids = child.into_inner();
            let pos1 = parse_position(kids.next().unwrap())?;
            let pos2 = parse_position(kids.next().unwrap())?;
            Ok(Position::Tuple(Box::new(pos1), Box::new(pos2)))
        }
        Rule::pos_group => {
            // "(" ~ position ~ ")"
            let mut kids = child.into_inner();
            parse_position(kids.next().unwrap())
        }
        Rule::pos_place_offset_paren | Rule::pos_place_offset => {
            // place ~ ("+" | "-") ~ expr ~ "," ~ expr
            let child_str = child.as_str();
            let mut kids = child.into_inner();
            let place = parse_place(kids.next().unwrap())?;
            let x = parse_expr(kids.next().unwrap())?;
            let y = parse_expr(kids.next().unwrap())?;
            // Determine op from the string (pest doesn't capture bare + or -)
            let op = if child_str.contains('+') {
                BinaryOp::Add
            } else {
                BinaryOp::Sub
            };
            Ok(Position::PlaceOffset(place, op, x, y))
        }
        Rule::pos_between => {
            // expr ~ ("between" | ...) ~ position ~ "and" ~ position
            let mut kids = child.into_inner();
            let factor = parse_expr(kids.next().unwrap())?;
            let pos1 = parse_position(kids.next().unwrap())?;
            let pos2 = parse_position(kids.next().unwrap())?;
            Ok(Position::Between(factor, Box::new(pos1), Box::new(pos2)))
        }
        Rule::pos_bracket => {
            // expr ~ "<" ~ position ~ "," ~ position ~ ">"
            let mut kids = child.into_inner();
            let factor = parse_expr(kids.next().unwrap())?;
            let pos1 = parse_position(kids.next().unwrap())?;
            let pos2 = parse_position(kids.next().unwrap())?;
            Ok(Position::Bracket(factor, Box::new(pos1), Box::new(pos2)))
        }
        Rule::pos_above_below => {
            // expr ~ above_below ~ position
            let mut kids = child.into_inner();
            let dist = parse_expr(kids.next().unwrap())?;
            let ab_pair = kids.next().unwrap();
            let ab = if ab_pair.as_str().trim() == "above" {
                AboveBelow::Above
            } else {
                AboveBelow::Below
            };
            let pos = parse_position(kids.next().unwrap())?;
            Ok(Position::AboveBelow(dist, ab, Box::new(pos)))
        }
        Rule::pos_left_right => {
            // expr ~ left_right_of ~ position
            let mut kids = child.into_inner();
            let dist = parse_expr(kids.next().unwrap())?;
            let lr_pair = kids.next().unwrap();
            let lr = if lr_pair.as_str().starts_with("left") {
                LeftRight::Left
            } else {
                LeftRight::Right
            };
            let pos = parse_position(kids.next().unwrap())?;
            Ok(Position::LeftRightOf(dist, lr, Box::new(pos)))
        }
        Rule::pos_heading => {
            // expr ~ "on"? ~ "heading" ~ (EDGEPT | expr) ~ ("of" | "from") ~ position
            let mut kids = child.into_inner();
            let dist = parse_expr(kids.next().unwrap())?;
            let heading_pair = kids.next().unwrap();
            let heading = if heading_pair.as_rule() == Rule::EDGEPT {
                HeadingDir::EdgePoint(parse_edgepoint(heading_pair)?)
            } else {
                HeadingDir::Expr(parse_expr(heading_pair)?)
            };
            let pos = parse_position(kids.next().unwrap())?;
            Ok(Position::Heading(dist, heading, Box::new(pos)))
        }
        Rule::pos_edgept_of => {
            // expr ~ EDGEPT ~ "of" ~ position
            let mut kids = child.into_inner();
            let dist = parse_expr(kids.next().unwrap())?;
            let ep = parse_edgepoint(kids.next().unwrap())?;
            let pos = parse_position(kids.next().unwrap())?;
            Ok(Position::EdgePointOf(dist, ep, Box::new(pos)))
        }
        Rule::pos_coords => {
            // expr ~ "," ~ expr
            let mut kids = child.into_inner();
            let x = parse_expr(kids.next().unwrap())?;
            let y = parse_expr(kids.next().unwrap())?;
            Ok(Position::Coords(x, y))
        }
        Rule::pos_place => {
            // place
            let mut kids = child.into_inner();
            let place = parse_place(kids.next().unwrap())?;
            Ok(Position::Place(place))
        }
        _ => Err(miette::miette!(
            "Unexpected position rule: {:?} in '{}'",
            child.as_rule(),
            pair_str
        )),
    }
}

fn parse_place(pair: Pair<Rule>) -> Result<Place, miette::Report> {
    let pair_str = pair.as_str();
    let mut inner = pair.into_inner().peekable();
    let first = match inner.peek() {
        Some(p) => p,
        None => return Err(miette::miette!("Empty place: {}", pair_str)),
    };

    match first.as_rule() {
        Rule::NTH => {
            // Grammar: NTH ~ "vertex" ~ "of" ~ object
            // "vertex" and "of" are literals, not captured
            let nth = parse_nth(inner.next().unwrap())?;
            // Next should be object directly
            if let Some(obj_pair) = inner.next() {
                let obj = parse_object(obj_pair)?;
                Ok(Place::Vertex(nth, obj))
            } else {
                Err(miette::miette!(
                    "Missing object in NTH vertex of object: {}",
                    pair_str
                ))
            }
        }
        Rule::EDGEPT => {
            // edgepoint of object
            // Grammar: EDGEPT ~ "of" ~ object
            // "of" is a literal and not captured
            let ep = parse_edgepoint(inner.next().unwrap())?;
            // Next should be object directly
            if let Some(obj_pair) = inner.next() {
                let obj = parse_object(obj_pair)?;
                Ok(Place::EdgePointOf(ep, obj))
            } else {
                // No object found - maybe this is just a bare edgepoint
                // Return as a placeholder object
                Err(miette::miette!(
                    "Missing object in EDGEPT of object: {} (edgepoint: {:?})",
                    pair_str,
                    ep
                ))
            }
        }
        Rule::object => {
            let obj = parse_object(inner.next().unwrap())?;
            if let Some(edge_pair) = inner.next() {
                // object.edge
                let ep = parse_edgepoint(edge_pair.into_inner().next().unwrap())?;
                Ok(Place::ObjectEdge(obj, ep))
            } else {
                // bare object
                Ok(Place::Object(obj))
            }
        }
        _ => Err(miette::miette!("Invalid place: {:?}", first.as_rule())),
    }
}

fn parse_object(pair: Pair<Rule>) -> Result<Object, miette::Report> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::objectname => Ok(Object::Named(parse_objectname(inner)?)),
        Rule::nth => Ok(Object::Nth(parse_nth(inner)?)),
        _ => Err(miette::miette!("Invalid object: {:?}", inner.as_rule())),
    }
}

fn parse_objectname(pair: Pair<Rule>) -> Result<ObjectName, miette::Report> {
    // Grammar: objectname = { "this" ~ dot_name* | PLACENAME ~ dot_name* }
    // "this" is a keyword - may not be captured. PLACENAME should be captured.
    let pair_str = pair.as_str();
    let mut inner = pair.into_inner().peekable();

    let base = if let Some(first) = inner.next() {
        if first.as_str() == "this" {
            ObjectNameBase::This
        } else {
            ObjectNameBase::PlaceName(first.as_str().to_string())
        }
    } else {
        // No children - check string directly
        if pair_str.trim().starts_with("this") {
            ObjectNameBase::This
        } else {
            // The string should be the PLACENAME (possibly with dot_names)
            let parts: Vec<&str> = pair_str.split('.').collect();
            ObjectNameBase::PlaceName(parts[0].to_string())
        }
    };

    let path: Vec<String> = inner
        .filter(|p| p.as_rule() == Rule::dot_name)
        .filter_map(|p| {
            // dot_name = { "." ~ PLACENAME }
            // PLACENAME might not be a child, just extract from string
            let s = p.as_str();
            let name = s.trim_start_matches('.');
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();

    Ok(ObjectName { base, path })
}

fn parse_nth(pair: Pair<Rule>) -> Result<Nth, miette::Report> {
    let pair_str = pair.as_str();
    let mut inner = pair.into_inner().peekable();

    // Handle case where nth has no inner children (like bare "previous")
    let first = match inner.next() {
        Some(p) => p,
        None => {
            // No children - check the string content
            let s = pair_str.trim();
            return if s == "previous" {
                Ok(Nth::Previous)
            } else if s == "first" {
                Ok(Nth::First(None))
            } else if s == "last" {
                Ok(Nth::Last(None))
            } else if s.starts_with("first") && s.contains("[]") {
                Ok(Nth::First(Some(NthClass::Sublist)))
            } else if s.starts_with("last") && s.contains("[]") {
                Ok(Nth::Last(Some(NthClass::Sublist)))
            } else if s.ends_with("st")
                || s.ends_with("nd")
                || s.ends_with("rd")
                || s.ends_with("th")
            {
                // Ordinal like "1st", "2nd", "3rd", "4th", etc.
                let num: u32 = s
                    .trim_end_matches(|c: char| !c.is_ascii_digit())
                    .parse()
                    .unwrap_or(1);
                Ok(Nth::Ordinal(num, false, None))
            } else {
                Err(miette::miette!("Invalid nth with no children: {}", s))
            };
        }
    };

    match first.as_rule() {
        Rule::NTH => {
            // Parse ordinal like "1st", "2nd", etc.
            let s = first.as_str();
            let num: u32 = s
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .unwrap_or(1);
            // Check for "last" keyword - it's consumed as a literal in pest grammar,
            // not as a child node, so we need to check the original pair string
            let is_last = pair_str.contains(" last ");
            let class = inner.next().map(|p| parse_nth_class(p)).transpose()?;
            tracing::debug!(num, is_last, ?class, pair_str, "parse_nth Ordinal");
            Ok(Nth::Ordinal(num, is_last, class))
        }
        Rule::CLASSNAME => {
            // This is "first CLASSNAME" or "last CLASSNAME" where first/last was the keyword
            // But actually the keyword wouldn't be captured... let me check
            let class = Some(NthClass::ClassName(parse_classname(first)?));
            // Check if this came from "first" or "last" by looking at the original string
            if pair_str.starts_with("first") {
                Ok(Nth::First(class))
            } else {
                Ok(Nth::Last(class))
            }
        }
        _ => {
            let s = first.as_str();
            match s {
                "first" => {
                    let class = inner.next().map(|p| parse_nth_class(p)).transpose()?;
                    Ok(Nth::First(class))
                }
                "last" => {
                    let class = inner.next().map(|p| parse_nth_class(p)).transpose()?;
                    Ok(Nth::Last(class))
                }
                "previous" => Ok(Nth::Previous),
                _ => Err(miette::miette!(
                    "Invalid nth: {} (rule: {:?})",
                    s,
                    first.as_rule()
                )),
            }
        }
    }
}

fn parse_nth_class(pair: Pair<Rule>) -> Result<NthClass, miette::Report> {
    if pair.as_str() == "[" || pair.as_str() == "]" {
        return Ok(NthClass::Sublist);
    }
    if pair.as_rule() == Rule::CLASSNAME {
        return Ok(NthClass::ClassName(parse_classname(pair)?));
    }
    // Try to find the actual classname
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::CLASSNAME {
            return Ok(NthClass::ClassName(parse_classname(inner)?));
        }
    }
    Ok(NthClass::Sublist)
}

fn parse_edgepoint(pair: Pair<Rule>) -> Result<EdgePoint, miette::Report> {
    match pair.as_str() {
        "north" | "n" => Ok(EdgePoint::North),
        "south" | "s" => Ok(EdgePoint::South),
        "east" | "e" => Ok(EdgePoint::East),
        "west" | "w" => Ok(EdgePoint::West),
        "start" => Ok(EdgePoint::Start),
        "end" => Ok(EdgePoint::End),
        "center" | "c" => Ok(EdgePoint::Center),
        "bottom" | "bot" => Ok(EdgePoint::Bottom),
        "top" | "t" => Ok(EdgePoint::Top),
        "left" => Ok(EdgePoint::Left),
        "right" => Ok(EdgePoint::Right),
        "ne" => Ok(EdgePoint::NorthEast),
        "nw" => Ok(EdgePoint::NorthWest),
        "se" => Ok(EdgePoint::SouthEast),
        "sw" => Ok(EdgePoint::SouthWest),
        s => Err(miette::miette!("Invalid edgepoint: {}", s)),
    }
}

fn parse_string(pair: Pair<Rule>) -> Result<String, miette::Report> {
    let s = pair.as_str();
    // Remove quotes and preserve backslash escape sequences
    // Note: C pikchr does NOT interpret \n, \t, etc. during parsing - it processes
    // them during rendering. Only \" and \\ are special during parsing.
    // cref: pik_append_txt (pikchr.c:2578-2588) - processes backslashes at render time
    let inner = &s[1..s.len() - 1];
    let mut result = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                chars.next();
                match next {
                    // Only process quote escapes during parsing
                    '"' => result.push('"'),
                    // Keep all other backslash sequences literal (including \\, \n, \t, etc.)
                    // They will be processed during rendering by process_backslash_escapes()
                    _ => {
                        result.push('\\');
                        result.push(next);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    Ok(result)
}
