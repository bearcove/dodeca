//! Typed views over the lossless CST.
//!
//! These are thin, zero-copy wrappers over [`crate::ResolvedNode`] (à la rust-analyzer's
//! `ast` layer): each wrapper is just a tagged node, and its accessor methods navigate the
//! CST. There is **no separate owned AST** — the engine and the LSP both evaluate directly
//! off these typed views, so there is a single source of truth. Leaf values (int/float/bool
//! literals, string-escape resolution, operator → enum) are decoded in the accessors.

use crate::ResolvedNode;
use crate::SyntaxKind::{self, *};

/// A typed view over a CST node of a known [`SyntaxKind`].
pub trait AstNode: Sized {
    fn can_cast(kind: SyntaxKind) -> bool;
    fn cast(node: ResolvedNode) -> Option<Self>;
    fn syntax(&self) -> &ResolvedNode;
}

macro_rules! ast_node {
    ($(#[$m:meta])* $name:ident = $kind:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone)]
        pub struct $name(ResolvedNode);
        impl AstNode for $name {
            fn can_cast(k: SyntaxKind) -> bool { k == SyntaxKind::$kind }
            fn cast(node: ResolvedNode) -> Option<Self> {
                if node.kind() == SyntaxKind::$kind { Some(Self(node)) } else { None }
            }
            fn syntax(&self) -> &ResolvedNode { &self.0 }
        }
    };
}

// ----- helpers on raw nodes -----

/// Iterate the child *nodes* that cast to `T`.
fn typed_children<T: AstNode>(node: &ResolvedNode) -> impl Iterator<Item = T> + '_ {
    node.children().filter_map(|c| T::cast(c.clone()))
}

/// First child node that casts to `T`.
fn typed_child<T: AstNode>(node: &ResolvedNode) -> Option<T> {
    typed_children(node).next()
}

/// Text of the first child token of `kind`, if any.
fn token_text(node: &ResolvedNode, kind: SyntaxKind) -> Option<&str> {
    node.children_with_tokens().find_map(|e| {
        let t = e.into_token()?;
        (t.kind() == kind).then(|| t.text())
    })
}

/// Kind of the first child token matching any operator kind in `kinds`.
fn first_token_kind(node: &ResolvedNode, kinds: &[SyntaxKind]) -> Option<SyntaxKind> {
    node.children_with_tokens().find_map(|e| {
        let t = e.into_token()?;
        kinds.contains(&t.kind()).then(|| t.kind())
    })
}

// ===== expressions =====

ast_node!(Literal = Literal);
ast_node!(VarRef = VarRef);
ast_node!(FieldExpr = FieldExpr);
ast_node!(IndexExpr = IndexExpr);
ast_node!(CallExpr = CallExpr);
ast_node!(FilterExpr = FilterExpr);
ast_node!(TestExpr = TestExpr);
ast_node!(TernaryExpr = TernaryExpr);
ast_node!(BinaryExpr = BinaryExpr);
ast_node!(UnaryExpr = UnaryExpr);
ast_node!(OptionalExpr = OptionalExpr);
ast_node!(ListLit = ListLit);
ast_node!(DictLit = DictLit);
ast_node!(ParenExpr = ParenExpr);
ast_node!(ArgList = ArgList);
ast_node!(Arg = Arg);
ast_node!(KwArg = KwArg);

/// Any expression node.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Literal),
    Var(VarRef),
    Field(FieldExpr),
    Index(IndexExpr),
    Call(CallExpr),
    Filter(FilterExpr),
    Test(TestExpr),
    Ternary(TernaryExpr),
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Optional(OptionalExpr),
    List(ListLit),
    Dict(DictLit),
    Paren(ParenExpr),
}

impl Expr {
    pub fn cast(node: ResolvedNode) -> Option<Expr> {
        Some(match node.kind() {
            SyntaxKind::Literal => Expr::Literal(self::Literal(node)),
            SyntaxKind::VarRef => Expr::Var(self::VarRef(node)),
            SyntaxKind::FieldExpr => Expr::Field(self::FieldExpr(node)),
            SyntaxKind::IndexExpr => Expr::Index(self::IndexExpr(node)),
            SyntaxKind::CallExpr => Expr::Call(self::CallExpr(node)),
            SyntaxKind::FilterExpr => Expr::Filter(self::FilterExpr(node)),
            SyntaxKind::TestExpr => Expr::Test(self::TestExpr(node)),
            SyntaxKind::TernaryExpr => Expr::Ternary(self::TernaryExpr(node)),
            SyntaxKind::BinaryExpr => Expr::Binary(self::BinaryExpr(node)),
            SyntaxKind::UnaryExpr => Expr::Unary(self::UnaryExpr(node)),
            SyntaxKind::OptionalExpr => Expr::Optional(self::OptionalExpr(node)),
            SyntaxKind::ListLit => Expr::List(self::ListLit(node)),
            SyntaxKind::DictLit => Expr::Dict(self::DictLit(node)),
            SyntaxKind::ParenExpr => Expr::Paren(self::ParenExpr(node)),
            _ => return None,
        })
    }

    pub fn syntax(&self) -> &ResolvedNode {
        match self {
            Expr::Literal(n) => n.syntax(),
            Expr::Var(n) => n.syntax(),
            Expr::Field(n) => n.syntax(),
            Expr::Index(n) => n.syntax(),
            Expr::Call(n) => n.syntax(),
            Expr::Filter(n) => n.syntax(),
            Expr::Test(n) => n.syntax(),
            Expr::Ternary(n) => n.syntax(),
            Expr::Binary(n) => n.syntax(),
            Expr::Unary(n) => n.syntax(),
            Expr::Optional(n) => n.syntax(),
            Expr::List(n) => n.syntax(),
            Expr::Dict(n) => n.syntax(),
            Expr::Paren(n) => n.syntax(),
        }
    }
}

fn first_expr(node: &ResolvedNode) -> Option<Expr> {
    node.children().find_map(|c| Expr::cast(c.clone()))
}

fn nth_expr(node: &ResolvedNode, n: usize) -> Option<Expr> {
    node.children().filter_map(|c| Expr::cast(c.clone())).nth(n)
}

/// The kind of literal a [`Literal`] node holds.
#[derive(Debug, Clone, PartialEq)]
pub enum LitValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    None,
}

impl Literal {
    pub fn value(&self) -> LitValue {
        let tok = self.0.first_token();
        let Some(tok) = tok else { return LitValue::None };
        match tok.kind() {
            Int => LitValue::Int(tok.text().parse().unwrap_or(0)),
            Float => LitValue::Float(tok.text().parse().unwrap_or(0.0)),
            True => LitValue::Bool(true),
            False => LitValue::Bool(false),
            NoneKw => LitValue::None,
            Str => LitValue::Str(unquote(tok.text())),
            _ => LitValue::None,
        }
    }
}

impl VarRef {
    pub fn name(&self) -> Option<&str> {
        token_text(&self.0, Ident)
    }
}

impl FieldExpr {
    pub fn base(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
    pub fn field(&self) -> Option<&str> {
        token_text(&self.0, Ident)
    }
}

impl IndexExpr {
    pub fn base(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
    /// `true` when this is a slice (`a[:n]` / `a[a:b]`) rather than a plain index.
    pub fn is_slice(&self) -> bool {
        self.0.children_with_tokens().any(|e| e.into_token().is_some_and(|t| t.kind() == Colon))
    }
    /// The index expression (for a plain index), i.e. the second expression child.
    pub fn index(&self) -> Option<Expr> {
        nth_expr(&self.0, 1)
    }
}

impl CallExpr {
    pub fn callee(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
    pub fn args(&self) -> Option<ArgList> {
        typed_child(&self.0)
    }
}

impl ArgList {
    pub fn positional(&self) -> impl Iterator<Item = Expr> + '_ {
        typed_children::<Arg>(&self.0).filter_map(|a| first_expr(a.syntax()))
    }
    pub fn keyword(&self) -> impl Iterator<Item = (String, Expr)> + '_ {
        typed_children::<KwArg>(&self.0)
            .filter_map(|k| Some((token_text(k.syntax(), Ident)?.to_owned(), first_expr(k.syntax())?)))
    }
}

impl FilterExpr {
    pub fn base(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
    pub fn name(&self) -> Option<&str> {
        // `expr | NAME` — the filter name is the Ident token directly under this node.
        token_text(&self.0, Ident)
    }
    pub fn args(&self) -> Option<ArgList> {
        typed_child(&self.0)
    }
}

impl TestExpr {
    pub fn base(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
    pub fn negated(&self) -> bool {
        self.0.children_with_tokens().any(|e| e.into_token().is_some_and(|t| t.kind() == NotKw))
    }
    pub fn name(&self) -> Option<&str> {
        token_text(&self.0, Ident)
    }
    pub fn args(&self) -> Option<ArgList> {
        typed_child(&self.0)
    }
}

impl TernaryExpr {
    /// `value if cond else otherwise`: the three expression children in source order.
    pub fn value(&self) -> Option<Expr> {
        nth_expr(&self.0, 0)
    }
    pub fn condition(&self) -> Option<Expr> {
        nth_expr(&self.0, 1)
    }
    pub fn otherwise(&self) -> Option<Expr> {
        nth_expr(&self.0, 2)
    }
}

/// A binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, FloorDiv, Mod, Pow, Concat,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or, In, NotIn,
}

impl BinaryExpr {
    pub fn lhs(&self) -> Option<Expr> {
        nth_expr(&self.0, 0)
    }
    pub fn rhs(&self) -> Option<Expr> {
        nth_expr(&self.0, 1)
    }
    pub fn op(&self) -> Option<BinOp> {
        // `not in` is the only binary carrying a `not` token; detect it first.
        let has = |k: SyntaxKind| {
            self.0.children_with_tokens().any(|e| e.into_token().is_some_and(|t| t.kind() == k))
        };
        if has(NotKw) && has(InKw) {
            return Some(BinOp::NotIn);
        }
        let k = first_token_kind(&self.0, &[
            Plus, Minus, Star, Slash, SlashSlash, Percent, StarStar, Tilde,
            EqEq, Neq, Lt, Le, Gt, Ge, AndKw, OrKw, InKw,
        ])?;
        Some(match k {
            Plus => BinOp::Add,
            Minus => BinOp::Sub,
            Star => BinOp::Mul,
            Slash => BinOp::Div,
            SlashSlash => BinOp::FloorDiv,
            Percent => BinOp::Mod,
            StarStar => BinOp::Pow,
            Tilde => BinOp::Concat,
            EqEq => BinOp::Eq,
            Neq => BinOp::Ne,
            Lt => BinOp::Lt,
            Le => BinOp::Le,
            Gt => BinOp::Gt,
            Ge => BinOp::Ge,
            AndKw => BinOp::And,
            OrKw => BinOp::Or,
            InKw => BinOp::In,
            _ => return None,
        })
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
}

impl UnaryExpr {
    pub fn op(&self) -> Option<UnOp> {
        match first_token_kind(&self.0, &[NotKw, Minus])? {
            NotKw => Some(UnOp::Not),
            Minus => Some(UnOp::Neg),
            _ => None,
        }
    }
    pub fn operand(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
}

impl OptionalExpr {
    pub fn operand(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
}

impl ListLit {
    pub fn elements(&self) -> impl Iterator<Item = Expr> + '_ {
        self.0.children().filter_map(|c| Expr::cast(c.clone()))
    }
}

impl DictLit {
    /// (key, value) pairs in source order.
    pub fn entries(&self) -> Vec<(Expr, Expr)> {
        let exprs: Vec<Expr> = self.0.children().filter_map(|c| Expr::cast(c.clone())).collect();
        exprs.chunks_exact(2).map(|c| (c[0].clone(), c[1].clone())).collect()
    }
}

impl ParenExpr {
    pub fn inner(&self) -> Option<Expr> {
        first_expr(&self.0)
    }
}

/// Resolve a quoted string literal to its value (strip quotes, process `\` escapes).
fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 {
        return raw.to_string();
    }
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_expr_str;

    /// Cast the single interpolation's expression out of a `{{ … }}` parse.
    fn expr_of(src: &str) -> Expr {
        let p = parse_expr_str(src);
        assert!(p.errors.is_empty(), "errors: {:?}", p.errors);
        // Template → Interpolation → Expr
        let interp = p.syntax().children().find(|n| n.kind() == Interpolation).unwrap();
        first_expr(interp).expect("an expression")
    }

    #[test]
    fn literals() {
        assert_eq!(matches!(expr_of("42"), Expr::Literal(l) if l.value() == LitValue::Int(42)), true);
        assert_eq!(matches!(expr_of("\"a\\nb\""), Expr::Literal(l) if l.value() == LitValue::Str("a\nb".into())), true);
        assert!(matches!(expr_of("true"), Expr::Literal(l) if l.value() == LitValue::Bool(true)));
    }

    #[test]
    fn binary_op_and_operands() {
        let Expr::Binary(b) = expr_of("a + b") else { panic!() };
        assert_eq!(b.op(), Some(BinOp::Add));
        assert!(matches!(b.lhs(), Some(Expr::Var(_))));
        assert!(matches!(b.rhs(), Some(Expr::Var(_))));
    }

    #[test]
    fn not_in_operator() {
        let Expr::Binary(b) = expr_of("x not in xs") else { panic!() };
        assert_eq!(b.op(), Some(BinOp::NotIn));
    }

    #[test]
    fn call_with_field_arg_and_kwarg() {
        let Expr::Call(c) = expr_of("f(a.b, k=c)") else { panic!() };
        let args = c.args().unwrap();
        assert_eq!(args.positional().count(), 1);
        let kw: Vec<_> = args.keyword().collect();
        assert_eq!(kw.len(), 1);
        assert_eq!(kw[0].0, "k");
        assert!(matches!(args.positional().next(), Some(Expr::Field(_))));
    }

    #[test]
    fn field_and_optional() {
        let Expr::Field(f) = expr_of("page.title") else { panic!() };
        assert_eq!(f.field(), Some("title"));
        assert!(matches!(expr_of("width?"), Expr::Optional(_)));
    }

    #[test]
    fn slice_detected() {
        let Expr::Index(i) = expr_of("xs[:3]") else { panic!() };
        assert!(i.is_slice());
    }
}
