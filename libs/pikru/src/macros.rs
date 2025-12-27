//! Macro expansion for pikchr
//!
//! Handles `define name { body }` and macro invocations.

use crate::ast::*;
use crate::parse;
use std::collections::HashMap;

const MAX_EXPANSION_DEPTH: usize = 10;

/// Macro definition
#[derive(Debug, Clone)]
struct MacroDef {
    body: String,
}

/// Expand all macros in a program
pub fn expand_macros(program: Program) -> Result<Program, miette::Report> {
    let mut macros: HashMap<String, MacroDef> = HashMap::new();
    let mut expanded_statements = Vec::new();

    // Process all statements, collecting defines and expanding macro calls
    for stmt in program.statements {
        process_statement(&mut macros, &mut expanded_statements, stmt, 0)?;
    }

    Ok(Program {
        statements: expanded_statements,
    })
}

/// Process a single statement, potentially expanding macros
fn process_statement(
    macros: &mut HashMap<String, MacroDef>,
    output: &mut Vec<Statement>,
    stmt: Statement,
    depth: usize,
) -> Result<(), miette::Report> {
    match stmt {
        Statement::Define(def) => {
            // Store the macro definition
            // Strip the outer braces from the body
            let body = def.body.trim();
            let body = if body.starts_with('{') && body.ends_with('}') {
                body[1..body.len() - 1].trim().to_string()
            } else {
                body.to_string()
            };

            macros.insert(def.name.clone(), MacroDef { body });
            // Don't add defines to output - they're just definitions
        }
        Statement::MacroCall(call) => {
            // Expand the macro call
            expand_macro_call(macros, output, &call, depth)?;
        }
        other => {
            // Regular statement - just pass through
            output.push(other);
        }
    }
    Ok(())
}

/// Expand a single macro call, adding results to output
fn expand_macro_call(
    macros: &mut HashMap<String, MacroDef>,
    output: &mut Vec<Statement>,
    call: &MacroCall,
    depth: usize,
) -> Result<(), miette::Report> {
    if depth >= MAX_EXPANSION_DEPTH {
        return Err(miette::miette!(
            "Macro expansion depth exceeded (max {}). Possible infinite recursion in macro '{}'",
            MAX_EXPANSION_DEPTH,
            call.name
        ));
    }

    // Look up the macro
    let macro_def = match macros.get(&call.name) {
        Some(def) => def.clone(), // Clone to avoid borrow issues
        None => {
            // Unknown macro - might be a built-in variable or identifier
            // Return without adding anything
            return Ok(());
        }
    };

    // Substitute arguments into the body
    let mut expanded_body = macro_def.body.clone();

    for (i, arg) in call.args.iter().enumerate() {
        let placeholder = format!("${}", i + 1);
        let replacement = macro_arg_to_string(arg);
        expanded_body = expanded_body.replace(&placeholder, &replacement);
    }

    // Parse the expanded body
    let expanded_program = parse::parse(&expanded_body)?;

    // Recursively process all statements from the expansion
    for stmt in expanded_program.statements {
        process_statement(macros, output, stmt, depth + 1)?;
    }

    Ok(())
}

/// Convert a macro argument to its string representation
fn macro_arg_to_string(arg: &MacroArg) -> String {
    match arg {
        MacroArg::String(s) => format!("\"{}\"", s),
        MacroArg::Expr(e) => expr_to_string(e),
        MacroArg::Ident(s) => s.clone(),
    }
}

/// Convert an expression to its string representation (for macro expansion)
fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Number(n) => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        Expr::Variable(v) => v.clone(),
        Expr::PlaceName(p) => p.clone(),
        Expr::ParenExpr(e) => format!("({})", expr_to_string(e)),
        Expr::BinaryOp(l, op, r) => {
            let op_str = match op {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
            };
            format!("{}{}{}", expr_to_string(l), op_str, expr_to_string(r))
        }
        Expr::UnaryOp(op, e) => {
            let op_str = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Pos => "+",
            };
            format!("{}{}", op_str, expr_to_string(e))
        }
        Expr::FuncCall(fc) => {
            let func_name = match fc.func {
                Function::Abs => "abs",
                Function::Cos => "cos",
                Function::Sin => "sin",
                Function::Int => "int",
                Function::Sqrt => "sqrt",
                Function::Max => "max",
                Function::Min => "min",
            };
            let args: Vec<String> = fc.args.iter().map(expr_to_string).collect();
            format!("{}({})", func_name, args.join(", "))
        }
        _ => String::new(),
    }
}
