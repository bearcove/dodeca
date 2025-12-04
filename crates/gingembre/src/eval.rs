//! Expression evaluator
//!
//! Evaluates template expressions against a context using facet_value::Value.

use super::ast::*;
use super::error::{
    TemplateSource, TypeError, UndefinedError, UnknownFieldError, UnknownFilterError,
    UnknownTestError,
};
use facet_value::{DestructuredRef, VArray, VObject, VString};
use miette::Result;
use std::collections::HashMap;

/// Re-export facet_value::Value as the template Value type
pub use facet_value::Value;

/// Helper trait to extend Value with template-specific operations
pub trait ValueExt {
    /// Check if the value is truthy (for conditionals)
    fn is_truthy(&self) -> bool;

    /// Get a human-readable type name
    fn type_name(&self) -> &'static str;

    /// Render the value to a string for output
    fn render_to_string(&self) -> String;

    /// Check if this value is marked as "safe" (should not be HTML-escaped)
    /// Note: facet_value doesn't have a Safe variant, so we track this separately
    fn is_safe(&self) -> bool {
        false // By default, values are not safe
    }
}

impl ValueExt for Value {
    fn is_truthy(&self) -> bool {
        match self.destructure_ref() {
            DestructuredRef::Null => false,
            DestructuredRef::Bool(b) => b,
            DestructuredRef::Number(n) => {
                if let Some(i) = n.to_i64() {
                    i != 0
                } else if let Some(f) = n.to_f64() {
                    f != 0.0
                } else {
                    true
                }
            }
            DestructuredRef::String(s) => !s.is_empty(),
            DestructuredRef::Bytes(b) => !b.is_empty(),
            DestructuredRef::Array(arr) => !arr.is_empty(),
            DestructuredRef::Object(obj) => !obj.is_empty(),
            DestructuredRef::DateTime(_) => true,
        }
    }

    fn type_name(&self) -> &'static str {
        match self.destructure_ref() {
            DestructuredRef::Null => "none",
            DestructuredRef::Bool(_) => "bool",
            DestructuredRef::Number(_) => "number",
            DestructuredRef::String(_) => "string",
            DestructuredRef::Bytes(_) => "bytes",
            DestructuredRef::Array(_) => "list",
            DestructuredRef::Object(_) => "dict",
            DestructuredRef::DateTime(_) => "datetime",
        }
    }

    fn render_to_string(&self) -> String {
        match self.destructure_ref() {
            DestructuredRef::Null => String::new(),
            DestructuredRef::Bool(b) => if b { "true" } else { "false" }.to_string(),
            DestructuredRef::Number(n) => {
                if let Some(i) = n.to_i64() {
                    i.to_string()
                } else if let Some(f) = n.to_f64() {
                    f.to_string()
                } else {
                    // Fallback for numbers that don't fit i64 or f64
                    "0".to_string()
                }
            }
            DestructuredRef::String(s) => s.to_string(),
            DestructuredRef::Bytes(b) => {
                // Render bytes as hex or base64
                format!("<bytes: {} bytes>", b.len())
            }
            DestructuredRef::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| v.render_to_string()).collect();
                format!("[{}]", items.join(", "))
            }
            DestructuredRef::Object(_) => "[object]".to_string(),
            DestructuredRef::DateTime(dt) => format!("{:?}", dt),
        }
    }
}

/// A "safe" wrapper that marks a value as not needing HTML escaping
#[derive(Debug, Clone)]
pub struct SafeValue(pub Value);

impl SafeValue {
    pub fn into_inner(self) -> Value {
        self.0
    }
}

/// A global function that can be called from templates
pub type GlobalFn = Box<dyn Fn(&[Value], &[(String, Value)]) -> Result<Value> + Send + Sync>;

/// Evaluation context (variables in scope)
#[derive(Clone)]
pub struct Context {
    /// Variable scopes (innermost last)
    scopes: Vec<HashMap<String, Value>>,
    /// Set of keys that are marked as "safe" (won't be HTML-escaped)
    safe_keys: std::collections::HashSet<String>,
    /// Global functions available in this context (shared via Arc)
    global_fns: std::sync::Arc<HashMap<String, std::sync::Arc<GlobalFn>>>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("scopes", &self.scopes)
            .field(
                "global_fns",
                &format!("<{} functions>", self.global_fns.len()),
            )
            .finish()
    }
}

impl Context {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            safe_keys: std::collections::HashSet::new(),
            global_fns: std::sync::Arc::new(HashMap::new()),
        }
    }

    /// Register a global function
    pub fn register_fn(&mut self, name: impl Into<String>, f: GlobalFn) {
        let fns = std::sync::Arc::make_mut(&mut self.global_fns);
        fns.insert(name.into(), std::sync::Arc::new(f));
    }

    /// Call a global function by name
    pub fn call_fn(
        &self,
        name: &str,
        args: &[Value],
        kwargs: &[(String, Value)],
    ) -> Option<Result<Value>> {
        self.global_fns.get(name).map(|f| f(args, kwargs))
    }

    /// Set a variable in the current scope
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.into(), value);
        }
    }

    /// Set a variable as "safe" (won't be HTML-escaped when rendered)
    pub fn set_safe(&mut self, name: impl Into<String>, value: Value) {
        let name = name.into();
        self.safe_keys.insert(name.clone());
        self.set(name, value);
    }

    /// Check if a variable is marked as safe
    pub fn is_safe(&self, name: &str) -> bool {
        self.safe_keys.contains(name)
    }

    /// Get a variable (searches all scopes)
    pub fn get(&self, name: &str) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value);
            }
        }
        None
    }

    /// Get all variable names (for error messages)
    pub fn available_vars(&self) -> Vec<String> {
        let mut vars: Vec<_> = self.scopes.iter().flat_map(|s| s.keys().cloned()).collect();
        vars.sort();
        vars.dedup();
        vars
    }

    /// Push a new scope
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the innermost scope
    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

/// Expression evaluator
pub struct Evaluator<'a> {
    ctx: &'a Context,
    source: &'a TemplateSource,
}

impl<'a> Evaluator<'a> {
    pub fn new(ctx: &'a Context, source: &'a TemplateSource) -> Self {
        Self { ctx, source }
    }

    /// Evaluate an expression to a value
    pub fn eval(&self, expr: &Expr) -> Result<Value> {
        match expr {
            Expr::Literal(lit) => self.eval_literal(lit),
            Expr::Var(ident) => self.eval_var(ident),
            Expr::Field(field) => self.eval_field(field),
            Expr::Index(index) => self.eval_index(index),
            Expr::Filter(filter) => self.eval_filter(filter),
            Expr::Binary(binary) => self.eval_binary(binary),
            Expr::Unary(unary) => self.eval_unary(unary),
            Expr::Call(call) => self.eval_call(call),
            Expr::Ternary(ternary) => self.eval_ternary(ternary),
            Expr::Test(test) => self.eval_test(test),
            Expr::MacroCall(_macro_call) => {
                // Macro calls are evaluated during rendering, not expression evaluation
                Ok(Value::NULL)
            }
        }
    }

    fn eval_literal(&self, lit: &Literal) -> Result<Value> {
        Ok(match lit {
            Literal::None(_) => Value::NULL,
            Literal::Bool(b) => Value::from(b.value),
            Literal::Int(i) => Value::from(i.value),
            Literal::Float(f) => Value::from(f.value),
            Literal::String(s) => Value::from(s.value.as_str()),
            Literal::List(l) => {
                let elements: Result<Vec<_>> = l.elements.iter().map(|e| self.eval(e)).collect();
                VArray::from_iter(elements?).into()
            }
            Literal::Dict(d) => {
                let mut obj = VObject::new();
                for (k, v) in &d.entries {
                    let key = self.eval(k)?.render_to_string();
                    let value = self.eval(v)?;
                    obj.insert(VString::from(key.as_str()), value);
                }
                obj.into()
            }
        })
    }

    fn eval_var(&self, ident: &Ident) -> Result<Value> {
        self.ctx.get(&ident.name).cloned().ok_or_else(|| {
            UndefinedError {
                name: ident.name.clone(),
                available: self.ctx.available_vars(),
                span: ident.span,
                src: self.source.named_source(),
            }
            .into()
        })
    }

    fn eval_field(&self, field: &FieldExpr) -> Result<Value> {
        let base = self.eval(&field.base)?;

        match base.destructure_ref() {
            DestructuredRef::Object(obj) => {
                obj.get(&field.field.name).cloned().ok_or_else(|| {
                    UnknownFieldError {
                        base_type: "dict".to_string(),
                        field: field.field.name.clone(),
                        known_fields: obj.keys().map(|k| k.to_string()).collect(),
                        span: field.field.span,
                        src: self.source.named_source(),
                    }
                    .into()
                })
            }
            _ => Err(TypeError {
                expected: "object or dict".to_string(),
                found: base.type_name().to_string(),
                context: "field access".to_string(),
                span: field.base.span(),
                src: self.source.named_source(),
            })?,
        }
    }

    fn eval_index(&self, index: &IndexExpr) -> Result<Value> {
        let base = self.eval(&index.base)?;
        let idx = self.eval(&index.index)?;

        match (base.destructure_ref(), idx.destructure_ref()) {
            (DestructuredRef::Array(arr), DestructuredRef::Number(n)) => {
                let i = n.to_i64().unwrap_or(0);
                let i = if i < 0 {
                    (arr.len() as i64 + i) as usize
                } else {
                    i as usize
                };
                arr.get(i).cloned().ok_or_else(|| {
                    TypeError {
                        expected: format!("index < {}", arr.len()),
                        found: format!("index {i}"),
                        context: "list index".to_string(),
                        span: index.index.span(),
                        src: self.source.named_source(),
                    }
                    .into()
                })
            }
            (DestructuredRef::Object(obj), DestructuredRef::String(key)) => {
                obj.get(key.as_str()).cloned().ok_or_else(|| {
                    UnknownFieldError {
                        base_type: "dict".to_string(),
                        field: key.to_string(),
                        known_fields: obj.keys().map(|k| k.to_string()).collect(),
                        span: index.index.span(),
                        src: self.source.named_source(),
                    }
                    .into()
                })
            }
            (DestructuredRef::String(s), DestructuredRef::Number(n)) => {
                let i = n.to_i64().unwrap_or(0);
                let len = s.len();
                let i = if i < 0 { (len as i64 + i) as usize } else { i as usize };
                s.as_str()
                    .chars()
                    .nth(i)
                    .map(|c| Value::from(c.to_string().as_str()))
                    .ok_or_else(|| {
                        TypeError {
                            expected: format!("index < {}", len),
                            found: format!("index {i}"),
                            context: "string index".to_string(),
                            span: index.index.span(),
                            src: self.source.named_source(),
                        }
                        .into()
                    })
            }
            _ => Err(TypeError {
                expected: "list, dict, or string".to_string(),
                found: base.type_name().to_string(),
                context: "index access".to_string(),
                span: index.base.span(),
                src: self.source.named_source(),
            })?,
        }
    }

    fn eval_filter(&self, filter: &FilterExpr) -> Result<Value> {
        let value = self.eval(&filter.expr)?;
        let args: Result<Vec<_>> = filter.args.iter().map(|a| self.eval(a)).collect();
        let args = args?;

        let kwargs: Result<Vec<(String, Value)>> = filter
            .kwargs
            .iter()
            .map(|(ident, expr)| Ok((ident.name.clone(), self.eval(expr)?)))
            .collect();
        let kwargs = kwargs?;

        apply_filter(
            &filter.filter.name,
            value,
            &args,
            &kwargs,
            filter.filter.span,
            self.source,
        )
    }

    fn eval_binary(&self, binary: &BinaryExpr) -> Result<Value> {
        // Short-circuit for and/or
        match binary.op {
            BinaryOp::And => {
                let left = self.eval(&binary.left)?;
                if !left.is_truthy() {
                    return Ok(left);
                }
                return self.eval(&binary.right);
            }
            BinaryOp::Or => {
                let left = self.eval(&binary.left)?;
                if left.is_truthy() {
                    return Ok(left);
                }
                return self.eval(&binary.right);
            }
            _ => {}
        }

        let left = self.eval(&binary.left)?;
        let right = self.eval(&binary.right)?;

        Ok(match binary.op {
            BinaryOp::Add => binary_add(&left, &right),
            BinaryOp::Sub => binary_sub(&left, &right),
            BinaryOp::Mul => binary_mul(&left, &right),
            BinaryOp::Div => binary_div(&left, &right),
            BinaryOp::FloorDiv => binary_floor_div(&left, &right),
            BinaryOp::Mod => binary_mod(&left, &right),
            BinaryOp::Pow => binary_pow(&left, &right),
            BinaryOp::Eq => Value::from(values_equal(&left, &right)),
            BinaryOp::Ne => Value::from(!values_equal(&left, &right)),
            BinaryOp::Lt => Value::from(
                compare_values(&left, &right)
                    .map(|o| o.is_lt())
                    .unwrap_or(false),
            ),
            BinaryOp::Le => Value::from(
                compare_values(&left, &right)
                    .map(|o| o.is_le())
                    .unwrap_or(false),
            ),
            BinaryOp::Gt => Value::from(
                compare_values(&left, &right)
                    .map(|o| o.is_gt())
                    .unwrap_or(false),
            ),
            BinaryOp::Ge => Value::from(
                compare_values(&left, &right)
                    .map(|o| o.is_ge())
                    .unwrap_or(false),
            ),
            BinaryOp::In => Value::from(value_in(&left, &right)),
            BinaryOp::NotIn => Value::from(!value_in(&left, &right)),
            BinaryOp::Concat => {
                Value::from(format!("{}{}", left.render_to_string(), right.render_to_string()).as_str())
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(), // Handled above
        })
    }

    fn eval_unary(&self, unary: &UnaryExpr) -> Result<Value> {
        let value = self.eval(&unary.expr)?;

        Ok(match unary.op {
            UnaryOp::Not => Value::from(!value.is_truthy()),
            UnaryOp::Neg => {
                match value.destructure_ref() {
                    DestructuredRef::Number(n) => {
                        if let Some(i) = n.to_i64() {
                            Value::from(-i)
                        } else if let Some(f) = n.to_f64() {
                            Value::from(-f)
                        } else {
                            Value::NULL
                        }
                    }
                    _ => Value::NULL,
                }
            }
            UnaryOp::Pos => {
                match value.destructure_ref() {
                    DestructuredRef::Number(_) => value,
                    _ => Value::NULL,
                }
            }
        })
    }

    fn eval_call(&self, call: &CallExpr) -> Result<Value> {
        // Evaluate arguments
        let args: Vec<Value> = call
            .args
            .iter()
            .map(|a| self.eval(a))
            .collect::<Result<Vec<_>>>()?;

        let kwargs: Vec<(String, Value)> = call
            .kwargs
            .iter()
            .map(|(ident, expr)| Ok((ident.name.clone(), self.eval(expr)?)))
            .collect::<Result<Vec<_>>>()?;

        // Check if this is a global function call
        if let Expr::Var(ident) = &*call.func {
            if let Some(result) = self.ctx.call_fn(&ident.name, &args, &kwargs) {
                return result;
            }
        }

        // Method calls on values (like .items(), etc.) - not implemented yet
        Ok(Value::NULL)
    }

    fn eval_ternary(&self, ternary: &TernaryExpr) -> Result<Value> {
        let condition = self.eval(&ternary.condition)?;
        if condition.is_truthy() {
            self.eval(&ternary.value)
        } else {
            self.eval(&ternary.otherwise)
        }
    }

    fn eval_test(&self, test: &TestExpr) -> Result<Value> {
        let value = self.eval(&test.expr)?;
        let args: Vec<Value> = test
            .args
            .iter()
            .map(|a| self.eval(a))
            .collect::<Result<Vec<_>>>()?;

        let result = match test.test_name.name.as_str() {
            // String tests
            "starting_with" | "startswith" => {
                if let (DestructuredRef::String(s), Some(prefix)) = (value.destructure_ref(), args.first()) {
                    if let DestructuredRef::String(p) = prefix.destructure_ref() {
                        s.as_str().starts_with(p.as_str())
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "ending_with" | "endswith" => {
                if let (DestructuredRef::String(s), Some(suffix)) = (value.destructure_ref(), args.first()) {
                    if let DestructuredRef::String(p) = suffix.destructure_ref() {
                        s.as_str().ends_with(p.as_str())
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "containing" | "contains" => {
                match value.destructure_ref() {
                    DestructuredRef::String(s) => {
                        if let Some(needle) = args.first() {
                            if let DestructuredRef::String(n) = needle.destructure_ref() {
                                s.as_str().contains(n.as_str())
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    DestructuredRef::Array(arr) => {
                        args.first()
                            .map(|needle| arr.iter().any(|item| values_equal(item, needle)))
                            .unwrap_or(false)
                    }
                    _ => false,
                }
            }
            // Type tests
            "defined" => !value.is_null(),
            "undefined" => value.is_null(),
            "none" => value.is_null(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => {
                if let DestructuredRef::Number(n) = value.destructure_ref() {
                    n.to_i64().is_some() && n.to_f64().map(|f| f.fract() == 0.0).unwrap_or(false)
                } else {
                    false
                }
            }
            "float" => {
                if let DestructuredRef::Number(n) = value.destructure_ref() {
                    n.to_f64().map(|f| f.fract() != 0.0).unwrap_or(false)
                } else {
                    false
                }
            }
            "mapping" | "dict" => value.is_object(),
            "iterable" | "sequence" => {
                matches!(value.destructure_ref(),
                    DestructuredRef::Array(_) | DestructuredRef::String(_) | DestructuredRef::Object(_))
            }
            // Value tests
            "odd" => {
                if let DestructuredRef::Number(n) = value.destructure_ref() {
                    n.to_i64().map(|i| i % 2 != 0).unwrap_or(false)
                } else {
                    false
                }
            }
            "even" => {
                if let DestructuredRef::Number(n) = value.destructure_ref() {
                    n.to_i64().map(|i| i % 2 == 0).unwrap_or(false)
                } else {
                    false
                }
            }
            "truthy" => value.is_truthy(),
            "falsy" => !value.is_truthy(),
            "empty" => match value.destructure_ref() {
                DestructuredRef::String(s) => s.is_empty(),
                DestructuredRef::Array(arr) => arr.is_empty(),
                DestructuredRef::Object(obj) => obj.is_empty(),
                _ => false,
            },
            // Comparison tests
            "eq" | "equalto" | "sameas" => args
                .first()
                .map(|other| values_equal(&value, other))
                .unwrap_or(false),
            "ne" => args
                .first()
                .map(|other| !values_equal(&value, other))
                .unwrap_or(false),
            "lt" | "lessthan" => {
                if let Some(other) = args.first() {
                    compare_values(&value, other).map(|o| o.is_lt()).unwrap_or(false)
                } else {
                    false
                }
            }
            "gt" | "greaterthan" => {
                if let Some(other) = args.first() {
                    compare_values(&value, other).map(|o| o.is_gt()).unwrap_or(false)
                } else {
                    false
                }
            }
            other => {
                return Err(UnknownTestError {
                    name: other.to_string(),
                    span: test.test_name.span,
                    src: self.source.named_source(),
                })?;
            }
        };

        Ok(Value::from(if test.negated { !result } else { result }))
    }
}

// === Binary operation helpers ===

fn binary_add(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                Value::from(ai + bi)
            } else if let (Some(af), Some(bf)) = (a.to_f64(), b.to_f64()) {
                Value::from(af + bf)
            } else {
                Value::NULL
            }
        }
        (DestructuredRef::String(a), DestructuredRef::String(b)) => {
            Value::from(format!("{}{}", a.as_str(), b.as_str()).as_str())
        }
        (DestructuredRef::Array(a), DestructuredRef::Array(b)) => {
            let mut result: Vec<Value> = a.iter().cloned().collect();
            result.extend(b.iter().cloned());
            VArray::from_iter(result).into()
        }
        _ => Value::NULL,
    }
}

fn binary_sub(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                Value::from(ai - bi)
            } else if let (Some(af), Some(bf)) = (a.to_f64(), b.to_f64()) {
                Value::from(af - bf)
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn binary_mul(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                Value::from(ai * bi)
            } else if let (Some(af), Some(bf)) = (a.to_f64(), b.to_f64()) {
                Value::from(af * bf)
            } else {
                Value::NULL
            }
        }
        (DestructuredRef::String(s), DestructuredRef::Number(n)) |
        (DestructuredRef::Number(n), DestructuredRef::String(s)) => {
            if let Some(count) = n.to_i64() {
                Value::from(s.as_str().repeat(count as usize).as_str())
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn binary_div(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(af), Some(bf)) = (a.to_f64(), b.to_f64()) {
                if bf != 0.0 {
                    Value::from(af / bf)
                } else {
                    Value::NULL
                }
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn binary_floor_div(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                if bi != 0 {
                    Value::from(ai / bi)
                } else {
                    Value::NULL
                }
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn binary_mod(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                if bi != 0 {
                    Value::from(ai % bi)
                } else {
                    Value::NULL
                }
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn binary_pow(left: &Value, right: &Value) -> Value {
    match (left.destructure_ref(), right.destructure_ref()) {
        (DestructuredRef::Number(a), DestructuredRef::Number(b)) => {
            if let (Some(ai), Some(bi)) = (a.to_i64(), b.to_i64()) {
                if bi >= 0 {
                    Value::from(ai.pow(bi as u32))
                } else {
                    Value::NULL
                }
            } else if let (Some(af), Some(bf)) = (a.to_f64(), b.to_f64()) {
                Value::from(af.powf(bf))
            } else {
                Value::NULL
            }
        }
        _ => Value::NULL,
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    a == b
}

fn compare_values(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    a.partial_cmp(b)
}

fn value_in(needle: &Value, haystack: &Value) -> bool {
    match haystack.destructure_ref() {
        DestructuredRef::Array(arr) => arr.iter().any(|v| values_equal(needle, v)),
        DestructuredRef::Object(obj) => {
            if let DestructuredRef::String(key) = needle.destructure_ref() {
                obj.contains_key(key.as_str())
            } else {
                false
            }
        }
        DestructuredRef::String(s) => {
            if let DestructuredRef::String(sub) = needle.destructure_ref() {
                s.as_str().contains(sub.as_str())
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Apply a built-in filter
fn apply_filter(
    name: &str,
    value: Value,
    args: &[Value],
    kwargs: &[(String, Value)],
    span: Span,
    source: &TemplateSource,
) -> Result<Value> {
    let known_filters = vec![
        "upper",
        "lower",
        "capitalize",
        "title",
        "trim",
        "length",
        "first",
        "last",
        "reverse",
        "sort",
        "join",
        "split",
        "default",
        "escape",
        "safe",
    ];

    // Helper to get kwarg value
    let get_kwarg =
        |key: &str| -> Option<&Value> { kwargs.iter().find(|(k, _)| k == key).map(|(_, v)| v) };

    Ok(match name {
        "upper" => Value::from(value.render_to_string().to_uppercase().as_str()),
        "lower" => Value::from(value.render_to_string().to_lowercase().as_str()),
        "capitalize" => {
            let s = value.render_to_string();
            let mut chars = s.chars();
            match chars.next() {
                None => Value::from(""),
                Some(first) => {
                    let result: String = first.to_uppercase().chain(chars).collect();
                    Value::from(result.as_str())
                }
            }
        }
        "title" => {
            let s = value.render_to_string();
            let result = s
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            Value::from(result.as_str())
        }
        "trim" => Value::from(value.render_to_string().trim()),
        "length" => match value.destructure_ref() {
            DestructuredRef::String(s) => Value::from(s.len() as i64),
            DestructuredRef::Array(arr) => Value::from(arr.len() as i64),
            DestructuredRef::Object(obj) => Value::from(obj.len() as i64),
            _ => Value::from(0i64),
        },
        "first" => match value.destructure_ref() {
            DestructuredRef::Array(arr) if !arr.is_empty() => arr.get(0).cloned().unwrap_or(Value::NULL),
            DestructuredRef::String(s) => s
                .as_str()
                .chars()
                .next()
                .map(|c| Value::from(c.to_string().as_str()))
                .unwrap_or(Value::NULL),
            _ => Value::NULL,
        },
        "last" => match value.destructure_ref() {
            DestructuredRef::Array(arr) if !arr.is_empty() => {
                arr.get(arr.len() - 1).cloned().unwrap_or(Value::NULL)
            }
            DestructuredRef::String(s) => s
                .as_str()
                .chars()
                .last()
                .map(|c| Value::from(c.to_string().as_str()))
                .unwrap_or(Value::NULL),
            _ => Value::NULL,
        },
        "reverse" => match value.destructure_ref() {
            DestructuredRef::Array(arr) => {
                let reversed: Vec<Value> = arr.iter().rev().cloned().collect();
                VArray::from_iter(reversed).into()
            }
            DestructuredRef::String(s) => {
                let reversed: String = s.as_str().chars().rev().collect();
                Value::from(reversed.as_str())
            }
            _ => value,
        },
        "sort" => match value.destructure_ref() {
            DestructuredRef::Array(arr) => {
                let mut items: Vec<Value> = arr.iter().cloned().collect();
                // Check for attribute= kwarg for sorting objects by field
                if let Some(attr_val) = get_kwarg("attribute") {
                    if let DestructuredRef::String(attr) = attr_val.destructure_ref() {
                        items.sort_by(|a, b| {
                            let a_val = if let DestructuredRef::Object(obj) = a.destructure_ref() {
                                obj.get(attr.as_str())
                            } else {
                                None
                            };
                            let b_val = if let DestructuredRef::Object(obj) = b.destructure_ref() {
                                obj.get(attr.as_str())
                            } else {
                                None
                            };
                            match (a_val, b_val) {
                                (Some(a), Some(b)) => {
                                    compare_values(a, b).unwrap_or(std::cmp::Ordering::Equal)
                                }
                                (Some(_), None) => std::cmp::Ordering::Less,
                                (None, Some(_)) => std::cmp::Ordering::Greater,
                                (None, None) => std::cmp::Ordering::Equal,
                            }
                        });
                    }
                } else {
                    items.sort_by(|a, b| compare_values(a, b).unwrap_or(std::cmp::Ordering::Equal));
                }
                VArray::from_iter(items).into()
            }
            _ => value,
        },
        "join" => {
            let sep = args
                .first()
                .map(|v| v.render_to_string())
                .unwrap_or_default();
            match value.destructure_ref() {
                DestructuredRef::Array(arr) => {
                    let strings: Vec<String> = arr.iter().map(|v| v.render_to_string()).collect();
                    Value::from(strings.join(&sep).as_str())
                }
                _ => value,
            }
        }
        "split" => {
            // Support both positional: split("/") and kwarg: split(pat="/")
            let pat = get_kwarg("pat")
                .map(|v| v.render_to_string())
                .or_else(|| args.first().map(|v| v.render_to_string()))
                .unwrap_or_else(|| " ".to_string());
            let s = value.render_to_string();
            let parts: Vec<Value> = s.split(&pat).map(|p| Value::from(p)).collect();
            VArray::from_iter(parts).into()
        }
        "default" => {
            // Support both positional: default("fallback") and kwarg: default(value="fallback")
            let default_val = get_kwarg("value")
                .cloned()
                .or_else(|| args.first().cloned())
                .unwrap_or(Value::NULL);

            if value.is_null() {
                default_val
            } else if let DestructuredRef::String(s) = value.destructure_ref() {
                if s.is_empty() {
                    default_val
                } else {
                    value
                }
            } else {
                value
            }
        }
        "escape" => {
            let s = value.render_to_string();
            let escaped = s
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#x27;");
            Value::from(escaped.as_str())
        }
        "safe" => {
            // For now, just return the value as-is
            // The renderer will need to track safe values separately
            value
        }
        _ => {
            return Err(UnknownFilterError {
                name: name.to_string(),
                known_filters: known_filters.into_iter().map(String::from).collect(),
                span,
                src: source.named_source(),
            }
            .into());
        }
    })
}
