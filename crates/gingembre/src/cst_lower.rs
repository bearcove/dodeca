//! Bridge: lower the cstree typed views (`gingembre_syntax::ast`) into the engine's
//! existing `ast::Expr`/`Node` so the new parser drives the unchanged eval/render.
//!
//! INTERIM: the correctness bar is rendered output, not AST parity, so spans are derived
//! from CST text ranges (more accurate than the old parser's) and need not match the old
//! ones. The end state is to evaluate directly off the typed views and delete `ast.rs`;
//! this bridge gets the engine green on the cstree parser first, then that follows.

use gingembre_syntax::ResolvedNode;
use gingembre_syntax::ast as cst;

use crate::ast::{self, BinaryOp, Expr, Ident, Span, UnaryOp};

/// Span covering a CST node.
fn sp(node: &ResolvedNode) -> Span {
    let r = node.text_range();
    ast::span(usize::from(r.start()), usize::from(r.len()))
}

fn ident(name: &str, node: &ResolvedNode) -> Ident {
    Ident { name: name.to_string(), span: sp(node) }
}

/// Lower a typed-CST expression to an engine `Expr`.
pub fn lower_expr(e: &cst::Expr) -> Expr {
    let node = e.syntax();
    let span = sp(node);
    match e {
        cst::Expr::Literal(l) => Expr::Literal(lower_literal(l, span)),
        cst::Expr::Var(v) => Expr::Var(Ident { name: v.name().unwrap_or_default().to_string(), span }),
        cst::Expr::Paren(p) => p.inner().map(|i| lower_expr(&i)).unwrap_or(Expr::Literal(ast::Literal::None(ast::NoneLit { span }))),
        cst::Expr::Field(f) => Expr::Field(ast::FieldExpr {
            base: Box::new(opt_expr(f.base(), span)),
            field: ident(f.field().unwrap_or_default(), node),
            span,
        }),
        cst::Expr::Index(i) => Expr::Index(ast::IndexExpr {
            base: Box::new(opt_expr(i.base(), span)),
            // Slices aren't a first-class engine Expr; use the (end) index expression, or
            // a None placeholder for a bare `[:]`. (Refine when porting eval off the CST.)
            index: Box::new(opt_expr(i.index(), span)),
            span,
        }),
        cst::Expr::Call(c) => {
            let (args, kwargs) = lower_args(c.args());
            Expr::Call(ast::CallExpr {
                func: Box::new(opt_expr(c.callee(), span)),
                args,
                kwargs,
                span,
            })
        }
        cst::Expr::Filter(f) => {
            let (args, kwargs) = lower_args(f.args());
            Expr::Filter(ast::FilterExpr {
                expr: Box::new(opt_expr(f.base(), span)),
                filter: ident(f.name().unwrap_or_default(), node),
                args,
                kwargs,
                span,
            })
        }
        cst::Expr::Test(t) => {
            let (args, _) = lower_args(t.args());
            Expr::Test(ast::TestExpr {
                expr: Box::new(opt_expr(t.base(), span)),
                test_name: ident(t.name().unwrap_or_default(), node),
                args,
                negated: t.negated(),
                span,
            })
        }
        cst::Expr::Ternary(t) => Expr::Ternary(ast::TernaryExpr {
            value: Box::new(opt_expr(t.value(), span)),
            condition: Box::new(opt_expr(t.condition(), span)),
            otherwise: Box::new(opt_expr(t.otherwise(), span)),
            span,
        }),
        cst::Expr::Binary(b) => Expr::Binary(ast::BinaryExpr {
            left: Box::new(opt_expr(b.lhs(), span)),
            op: lower_binop(b.op()),
            right: Box::new(opt_expr(b.rhs(), span)),
            span,
        }),
        cst::Expr::Unary(u) => Expr::Unary(ast::UnaryExpr {
            op: match u.op() {
                Some(cst::UnOp::Not) => UnaryOp::Not,
                _ => UnaryOp::Neg,
            },
            expr: Box::new(opt_expr(u.operand(), span)),
            span,
        }),
        cst::Expr::Optional(o) => Expr::Optional(ast::OptionalExpr {
            expr: Box::new(opt_expr(o.operand(), span)),
            span,
        }),
        cst::Expr::List(l) => Expr::Literal(ast::Literal::List(ast::ListLit {
            elements: l.elements().map(|x| lower_expr(&x)).collect(),
            span,
        })),
        cst::Expr::Dict(d) => Expr::Literal(ast::Literal::Dict(ast::DictLit {
            entries: d.entries().iter().map(|(k, v)| (lower_expr(k), lower_expr(v))).collect(),
            span,
        })),
    }
}

fn opt_expr(e: Option<cst::Expr>, span: Span) -> Expr {
    e.map(|x| lower_expr(&x))
        .unwrap_or(Expr::Literal(ast::Literal::None(ast::NoneLit { span })))
}

fn lower_literal(l: &cst::Literal, span: Span) -> ast::Literal {
    match l.value() {
        cst::LitValue::Str(s) => ast::Literal::String(ast::StringLit { value: s, span }),
        cst::LitValue::Int(i) => ast::Literal::Int(ast::IntLit { value: i, span }),
        cst::LitValue::Float(f) => ast::Literal::Float(ast::FloatLit { value: f, span }),
        cst::LitValue::Bool(b) => ast::Literal::Bool(ast::BoolLit { value: b, span }),
        cst::LitValue::None => ast::Literal::None(ast::NoneLit { span }),
    }
}

fn lower_args(args: Option<cst::ArgList>) -> (Vec<Expr>, Vec<(Ident, Expr)>) {
    let Some(args) = args else { return (Vec::new(), Vec::new()) };
    let pos = args.positional().map(|e| lower_expr(&e)).collect();
    let kw = args
        .keyword()
        .map(|(name, e)| {
            let span = sp(e.syntax());
            (Ident { name, span }, lower_expr(&e))
        })
        .collect();
    (pos, kw)
}

fn lower_binop(op: Option<cst::BinOp>) -> BinaryOp {
    match op {
        Some(cst::BinOp::Add) => BinaryOp::Add,
        Some(cst::BinOp::Sub) => BinaryOp::Sub,
        Some(cst::BinOp::Mul) => BinaryOp::Mul,
        Some(cst::BinOp::Div) => BinaryOp::Div,
        Some(cst::BinOp::FloorDiv) => BinaryOp::FloorDiv,
        Some(cst::BinOp::Mod) => BinaryOp::Mod,
        Some(cst::BinOp::Pow) => BinaryOp::Pow,
        Some(cst::BinOp::Concat) => BinaryOp::Concat,
        Some(cst::BinOp::Eq) => BinaryOp::Eq,
        Some(cst::BinOp::Ne) => BinaryOp::Ne,
        Some(cst::BinOp::Lt) => BinaryOp::Lt,
        Some(cst::BinOp::Le) => BinaryOp::Le,
        Some(cst::BinOp::Gt) => BinaryOp::Gt,
        Some(cst::BinOp::Ge) => BinaryOp::Ge,
        Some(cst::BinOp::And) => BinaryOp::And,
        Some(cst::BinOp::Or) => BinaryOp::Or,
        Some(cst::BinOp::In) => BinaryOp::In,
        Some(cst::BinOp::NotIn) => BinaryOp::NotIn,
        None => BinaryOp::Add,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gingembre_syntax::SyntaxKind;
    use gingembre_syntax::ast::AstNode;

    /// Lower the expression inside a `{{ … }}` and check it lowers without panicking,
    /// matching the expected engine Expr variant.
    fn lower(src: &str) -> Expr {
        let p = gingembre_syntax::parse(&format!("{{{{ {src} }}}}"));
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let interp = p.syntax().children().find(|n| n.kind() == SyntaxKind::Interpolation).unwrap();
        let cst_expr = cst::Interpolation::cast(interp.clone()).unwrap().expr().unwrap();
        lower_expr(&cst_expr)
    }

    #[test]
    fn lowers_core_exprs() {
        assert!(matches!(lower("42"), Expr::Literal(ast::Literal::Int(i)) if i.value == 42));
        assert!(matches!(lower("a + b"), Expr::Binary(b) if b.op == BinaryOp::Add));
        assert!(matches!(lower("x not in xs"), Expr::Binary(b) if b.op == BinaryOp::NotIn));
        assert!(matches!(lower("page.title"), Expr::Field(_)));
        assert!(matches!(lower("f(a.b, k=c)"), Expr::Call(c) if c.args.len() == 1 && c.kwargs.len() == 1));
        assert!(matches!(lower("x | upper | safe"), Expr::Filter(_)));
        assert!(matches!(lower("width?"), Expr::Optional(_)));
        assert!(matches!(lower("a if c else b"), Expr::Ternary(_)));
        assert!(matches!(lower("[1, 2]"), Expr::Literal(ast::Literal::List(l)) if l.elements.len() == 2));
    }
}
