//! Expression evaluator
//!
//! Evaluates template expressions against a context using facet_value::Value.

use super::ast::*;
use super::error::{
    TemplateSource, TypeError, UndefinedError, UnknownFieldError, UnknownFilterError,
    UnknownTestError,
};
use facet_value::{DestructuredRef, VArray, VObject, VString};
use futures::future::BoxFuture;
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
    fn is_safe(&self) -> bool;
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
            DestructuredRef::QName(_) => true,
            DestructuredRef::Uuid(_) => true,
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
            DestructuredRef::QName(_) => "qname",
            DestructuredRef::Uuid(_) => "uuid",
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
            DestructuredRef::QName(qn) => format!("{:?}", qn),
            DestructuredRef::Uuid(uuid) => format!("{:?}", uuid),
        }
    }

    fn is_safe(&self) -> bool {
        // Check if this is a safe string using VSafeString's flag
        self.as_string().is_some_and(|s| s.is_safe())
    }
}

use crate::lazy::{DataPath, DataResolver, LazyValue};

/// A global function that can be called from templates.
/// Functions receive resolved (concrete) values and return a future that resolves to a value.
/// This allows functions to make async calls (like RPC to the host).
pub type GlobalFn =
    Box<dyn Fn(&[Value], &[(String, Value)]) -> BoxFuture<'static, Result<Value>> + Send + Sync>;

/// Evaluation context (variables in scope)
///
/// The context stores [`LazyValue`]s, which can be either concrete values or
/// lazy references that resolve on demand. This enables fine-grained dependency
/// tracking for incremental computation.
#[derive(Clone)]
pub struct Context {
    /// Variable scopes (innermost last)
    scopes: Vec<HashMap<String, LazyValue>>,
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
    ) -> Option<BoxFuture<'static, Result<Value>>> {
        self.global_fns.get(name).map(|f| f(args, kwargs))
    }

    /// Set a variable in the current scope (concrete value)
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<LazyValue>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.into(), value.into());
        }
    }

    /// Get all variable names across all scopes
    pub fn variable_names(&self) -> Vec<&str> {
        self.scopes
            .iter()
            .flat_map(|scope| scope.keys().map(String::as_str))
            .collect()
    }

    /// Set a variable as "safe" (won't be HTML-escaped when rendered)
    /// If the value is a string, it will be converted to a VSafeString
    pub fn set_safe(&mut self, name: impl Into<String>, value: Value) {
        // Convert string values to safe strings
        let safe_value = if let Some(s) = value.as_string() {
            s.clone().into_safe().into_value()
        } else {
            value
        };
        self.set(name, safe_value);
    }

    /// Set a lazy data resolver as the "data" variable.
    ///
    /// This creates a lazy value at the root path that will resolve fields
    /// on demand, enabling fine-grained dependency tracking.
    pub fn set_data_resolver(&mut self, resolver: std::sync::Arc<dyn DataResolver>) {
        self.set("data", LazyValue::lazy(resolver, DataPath::root()));
    }

    /// Get a variable (searches all scopes)
    pub fn get(&self, name: &str) -> Option<&LazyValue> {
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
///
/// The evaluator returns [`LazyValue`]s, which may be either concrete or lazy.
/// Field and index access on lazy values extends the path without resolving.
/// Operations that need concrete values (arithmetic, comparison) force resolution.
pub struct Evaluator<'a> {
    ctx: &'a Context,
    source: &'a TemplateSource,
}

impl<'a> Evaluator<'a> {
    pub fn new(ctx: &'a Context, source: &'a TemplateSource) -> Self {
        Self { ctx, source }
    }

    /// Evaluate an expression to a (possibly lazy) value
    pub fn eval<'b>(&'b self, expr: &'b Expr) -> BoxFuture<'b, Result<LazyValue>> {
        Box::pin(async move {
            match expr {
                Expr::Literal(lit) => self.eval_literal(lit).await,
                Expr::Var(ident) => self.eval_var(ident),
                Expr::Field(field) => self.eval_field(field).await,
                Expr::Index(index) => self.eval_index(index).await,
                Expr::Filter(filter) => self.eval_filter(filter).await,
                Expr::Binary(binary) => self.eval_binary(binary).await,
                Expr::Unary(unary) => self.eval_unary(unary).await,
                Expr::Call(call) => self.eval_call(call).await,
                Expr::Ternary(ternary) => self.eval_ternary(ternary).await,
                Expr::Test(test) => self.eval_test(test).await,
                Expr::MacroCall(_macro_call) => {
                    // Macro calls are evaluated during rendering, not expression evaluation
                    Ok(LazyValue::concrete(Value::NULL))
                }
            }
        })
    }

    /// Evaluate and resolve to a concrete value (forces lazy resolution)
    pub async fn eval_concrete(&self, expr: &Expr) -> Result<Value> {
        self.eval(expr).await?.resolve().await
    }

    async fn eval_literal(&self, lit: &Literal) -> Result<LazyValue> {
        Ok(LazyValue::concrete(match lit {
            Literal::None(_) => Value::NULL,
            Literal::Bool(b) => Value::from(b.value),
            Literal::Int(i) => Value::from(i.value),
            Literal::Float(f) => Value::from(f.value),
            Literal::String(s) => Value::from(s.value.as_str()),
            Literal::List(l) => {
                // List elements are resolved to concrete values
                let mut elements = Vec::with_capacity(l.elements.len());
                for e in &l.elements {
                    elements.push(self.eval_concrete(e).await?);
                }
                VArray::from_iter(elements).into()
            }
            Literal::Dict(d) => {
                let mut obj = VObject::new();
                for (k, v) in &d.entries {
                    let key = self.eval(k).await?.render_to_string().await;
                    let value = self.eval_concrete(v).await?;
                    obj.insert(VString::from(key.as_str()), value);
                }
                obj.into()
            }
        }))
    }

    fn eval_var(&self, ident: &Ident) -> Result<LazyValue> {
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

    async fn eval_field(&self, field: &FieldExpr) -> Result<LazyValue> {
        let base = self.eval(&field.base).await?;
        // Use LazyValue's field method - extends path for lazy, normal access for concrete
        base.field(&field.field.name, field.field.span, self.source)
    }

    async fn eval_index(&self, index: &IndexExpr) -> Result<LazyValue> {
        let base = self.eval(&index.base).await?;
        let idx = self.eval(&index.index).await?;

        // For lazy base, we need to resolve the index to get a concrete key/index
        match &base {
            LazyValue::Lazy { .. } => {
                // Resolve the index to get the key
                let idx_resolved = idx.resolve().await?;
                match idx_resolved.destructure_ref() {
                    DestructuredRef::Number(n) => {
                        let i = n.to_i64().unwrap_or(0);
                        base.index(i, index.index.span(), self.source)
                    }
                    DestructuredRef::String(s) => {
                        base.index_str(s.as_str(), index.index.span(), self.source)
                    }
                    _ => Err(TypeError {
                        expected: "number or string".to_string(),
                        found: idx.type_name().to_string(),
                        context: "index".to_string(),
                        span: index.index.span(),
                        src: self.source.named_source(),
                    }
                    .into()),
                }
            }
            LazyValue::Concrete(base_val) => {
                // Original concrete logic
                let idx_resolved = idx.resolve().await?;
                match (base_val.destructure_ref(), idx_resolved.destructure_ref()) {
                    (DestructuredRef::Array(arr), DestructuredRef::Number(n)) => {
                        let i = n.to_i64().unwrap_or(0);
                        let i = if i < 0 {
                            (arr.len() as i64 + i) as usize
                        } else {
                            i as usize
                        };
                        arr.get(i).cloned().map(LazyValue::concrete).ok_or_else(|| {
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
                    (DestructuredRef::Object(obj), DestructuredRef::String(key)) => obj
                        .get(key.as_str())
                        .cloned()
                        .map(LazyValue::concrete)
                        .ok_or_else(|| {
                            UnknownFieldError {
                                base_type: "dict".to_string(),
                                field: key.to_string(),
                                known_fields: obj.keys().map(|k| k.to_string()).collect(),
                                span: index.index.span(),
                                src: self.source.named_source(),
                            }
                            .into()
                        }),
                    (DestructuredRef::String(s), DestructuredRef::Number(n)) => {
                        let i = n.to_i64().unwrap_or(0);
                        let len = s.len();
                        let i = if i < 0 {
                            (len as i64 + i) as usize
                        } else {
                            i as usize
                        };
                        s.as_str()
                            .chars()
                            .nth(i)
                            .map(|c| LazyValue::concrete(Value::from(c.to_string().as_str())))
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
        }
    }

    async fn eval_filter(&self, filter: &FilterExpr) -> Result<LazyValue> {
        // Filters always work on concrete values
        let value = self.eval_concrete(&filter.expr).await?;
        let mut args = Vec::with_capacity(filter.args.len());
        for a in &filter.args {
            args.push(self.eval_concrete(a).await?);
        }

        let mut kwargs = Vec::with_capacity(filter.kwargs.len());
        for (ident, expr) in &filter.kwargs {
            kwargs.push((ident.name.clone(), self.eval_concrete(expr).await?));
        }

        apply_filter(
            &filter.filter.name,
            value,
            &args,
            &kwargs,
            filter.filter.span,
            self.source,
        )
        .map(LazyValue::concrete)
    }

    async fn eval_binary(&self, binary: &BinaryExpr) -> Result<LazyValue> {
        // Short-circuit for and/or - these can stay lazy
        match binary.op {
            BinaryOp::And => {
                let left = self.eval(&binary.left).await?;
                if !left.is_truthy().await {
                    return Ok(left);
                }
                return self.eval(&binary.right).await;
            }
            BinaryOp::Or => {
                let left = self.eval(&binary.left).await?;
                if left.is_truthy().await {
                    return Ok(left);
                }
                return self.eval(&binary.right).await;
            }
            _ => {}
        }

        // All other binary ops need concrete values
        let left = self.eval_concrete(&binary.left).await?;
        let right = self.eval_concrete(&binary.right).await?;

        Ok(LazyValue::concrete(match binary.op {
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
            BinaryOp::Concat => Value::from(
                format!("{}{}", left.render_to_string(), right.render_to_string()).as_str(),
            ),
            BinaryOp::And | BinaryOp::Or => unreachable!(), // Handled above
        }))
    }

    async fn eval_unary(&self, unary: &UnaryExpr) -> Result<LazyValue> {
        // Unary ops need concrete values
        let value = self.eval_concrete(&unary.expr).await?;

        Ok(LazyValue::concrete(match unary.op {
            UnaryOp::Not => Value::from(!value.is_truthy()),
            UnaryOp::Neg => match value.destructure_ref() {
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
            },
            UnaryOp::Pos => match value.destructure_ref() {
                DestructuredRef::Number(_) => value,
                _ => Value::NULL,
            },
        }))
    }

    async fn eval_call(&self, call: &CallExpr) -> Result<LazyValue> {
        // Function calls require concrete argument values
        let mut args = Vec::with_capacity(call.args.len());
        for a in &call.args {
            args.push(self.eval_concrete(a).await?);
        }

        let mut kwargs = Vec::with_capacity(call.kwargs.len());
        for (ident, expr) in &call.kwargs {
            kwargs.push((ident.name.clone(), self.eval_concrete(expr).await?));
        }

        // Check if this is a global function call
        if let Expr::Var(ident) = &*call.func
            && let Some(result_fut) = self.ctx.call_fn(&ident.name, &args, &kwargs)
        {
            return result_fut.await.map(LazyValue::concrete);
        }

        // Method calls on values (like .items(), etc.) - not implemented yet
        Ok(LazyValue::concrete(Value::NULL))
    }

    async fn eval_ternary(&self, ternary: &TernaryExpr) -> Result<LazyValue> {
        let condition = self.eval(&ternary.condition).await?;
        if condition.is_truthy().await {
            self.eval(&ternary.value).await
        } else {
            self.eval(&ternary.otherwise).await
        }
    }

    async fn eval_test(&self, test: &TestExpr) -> Result<LazyValue> {
        // Tests require concrete values
        let value = self.eval_concrete(&test.expr).await?;
        let mut args = Vec::with_capacity(test.args.len());
        for a in &test.args {
            args.push(self.eval_concrete(a).await?);
        }

        let result = match test.test_name.name.as_str() {
            // String tests
            "starting_with" | "startswith" => {
                if let (DestructuredRef::String(s), Some(prefix)) =
                    (value.destructure_ref(), args.first())
                {
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
                if let (DestructuredRef::String(s), Some(suffix)) =
                    (value.destructure_ref(), args.first())
                {
                    if let DestructuredRef::String(p) = suffix.destructure_ref() {
                        s.as_str().ends_with(p.as_str())
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "containing" | "contains" => match value.destructure_ref() {
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
                DestructuredRef::Array(arr) => args
                    .first()
                    .map(|needle| arr.iter().any(|item| values_equal(item, needle)))
                    .unwrap_or(false),
                _ => false,
            },
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
                matches!(
                    value.destructure_ref(),
                    DestructuredRef::Array(_)
                        | DestructuredRef::String(_)
                        | DestructuredRef::Object(_)
                )
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
                    compare_values(&value, other)
                        .map(|o| o.is_lt())
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            "gt" | "greaterthan" => {
                if let Some(other) = args.first() {
                    compare_values(&value, other)
                        .map(|o| o.is_gt())
                        .unwrap_or(false)
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

        Ok(LazyValue::concrete(Value::from(if test.negated {
            !result
        } else {
            result
        })))
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
        (DestructuredRef::String(s), DestructuredRef::Number(n))
        | (DestructuredRef::Number(n), DestructuredRef::String(s)) => {
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

/// Helper for selectattr/rejectattr filters
fn filter_by_attr<'a>(
    value: &Value,
    args: &[Value],
    get_kwarg: impl Fn(&str) -> Option<&'a Value>,
    reject: bool,
) -> Value {
    match value.destructure_ref() {
        DestructuredRef::Array(arr) => {
            // args[0] = attribute name (required)
            // args[1] = test name (optional, default "truthy")
            // args[2] = test value (optional, for comparison tests)
            let attr_name = match args.first().and_then(|v| v.as_string()) {
                Some(s) => s.to_string(),
                None => return value.clone(),
            };

            let test_name = args
                .get(1)
                .and_then(|v| v.as_string())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "truthy".to_string());

            let test_value = args.get(2).cloned().or_else(|| get_kwarg("value").cloned());

            let filtered: Vec<Value> = arr
                .iter()
                .filter(|item| {
                    let attr_val = if let DestructuredRef::Object(obj) = item.destructure_ref() {
                        obj.get(attr_name.as_str()).cloned()
                    } else {
                        None
                    };

                    let passes = match test_name.as_str() {
                        "truthy" => attr_val.as_ref().map(|v| v.is_truthy()).unwrap_or(false),
                        "falsy" => attr_val.as_ref().map(|v| !v.is_truthy()).unwrap_or(true),
                        "defined" => attr_val.is_some(),
                        "undefined" => attr_val.is_none(),
                        "none" => attr_val.as_ref().map(|v| v.is_null()).unwrap_or(true),
                        "eq" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => values_equal(a, b),
                            _ => false,
                        },
                        "ne" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => !values_equal(a, b),
                            _ => true,
                        },
                        "gt" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                compare_values(a, b) == Some(std::cmp::Ordering::Greater)
                            }
                            _ => false,
                        },
                        "ge" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                matches!(
                                    compare_values(a, b),
                                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                                )
                            }
                            _ => false,
                        },
                        "lt" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                compare_values(a, b) == Some(std::cmp::Ordering::Less)
                            }
                            _ => false,
                        },
                        "le" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                matches!(
                                    compare_values(a, b),
                                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                                )
                            }
                            _ => false,
                        },
                        "starting_with" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                a.render_to_string().starts_with(&b.render_to_string())
                            }
                            _ => false,
                        },
                        "ending_with" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                a.render_to_string().ends_with(&b.render_to_string())
                            }
                            _ => false,
                        },
                        "containing" => match (&attr_val, &test_value) {
                            (Some(a), Some(b)) => {
                                a.render_to_string().contains(&b.render_to_string())
                            }
                            _ => false,
                        },
                        _ => attr_val.as_ref().map(|v| v.is_truthy()).unwrap_or(false),
                    };

                    if reject { !passes } else { passes }
                })
                .cloned()
                .collect();

            VArray::from_iter(filtered).into()
        }
        _ => value.clone(),
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
        "typeof",
        "slice",
        "map",
        "selectattr",
        "rejectattr",
        "groupby",
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
            DestructuredRef::Array(arr) if !arr.is_empty() => {
                arr.get(0).cloned().unwrap_or(Value::NULL)
            }
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
            let parts: Vec<Value> = s.split(&pat).map(Value::from).collect();
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
                if s.is_empty() { default_val } else { value }
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
            // Convert string to safe string using VSafeString
            if let Some(s) = value.as_string() {
                s.clone().into_safe().into_value()
            } else {
                // Non-strings can't be marked safe, return as-is
                value
            }
        }
        "typeof" => {
            // Return the type name of the value
            Value::from(value.type_name())
        }
        "slice" => {
            // Slice a list: slice(start=0, end=N) or slice(0, N)
            match value.destructure_ref() {
                DestructuredRef::Array(arr) => {
                    let start = get_kwarg("start")
                        .and_then(|v| v.as_number().and_then(|n| n.to_i64()))
                        .or_else(|| {
                            args.first()
                                .and_then(|v| v.as_number().and_then(|n| n.to_i64()))
                        })
                        .unwrap_or(0)
                        .max(0) as usize;
                    let end = get_kwarg("end")
                        .and_then(|v| v.as_number().and_then(|n| n.to_i64()))
                        .or_else(|| {
                            args.get(1)
                                .and_then(|v| v.as_number().and_then(|n| n.to_i64()))
                        })
                        .map(|e| e.max(0) as usize)
                        .unwrap_or(arr.len());
                    let end = end.min(arr.len());
                    let start = start.min(end);
                    VArray::from_iter(arr.iter().skip(start).take(end - start).cloned()).into()
                }
                _ => value,
            }
        }
        "map" => {
            // Extract an attribute from each item: map(attribute="field")
            match value.destructure_ref() {
                DestructuredRef::Array(arr) => {
                    if let Some(attr) = get_kwarg("attribute").and_then(|v| v.as_string()) {
                        let mapped: Vec<Value> = arr
                            .iter()
                            .filter_map(|item| {
                                if let DestructuredRef::Object(obj) = item.destructure_ref() {
                                    obj.get(attr.as_str()).cloned()
                                } else {
                                    None
                                }
                            })
                            .collect();
                        VArray::from_iter(mapped).into()
                    } else {
                        value
                    }
                }
                _ => value,
            }
        }
        "selectattr" => {
            // Filter items where attribute passes a test: selectattr("field", "eq", value)
            filter_by_attr(&value, args, get_kwarg, false)
        }
        "rejectattr" => {
            // Filter items where attribute fails a test: rejectattr("field", "eq", value)
            filter_by_attr(&value, args, get_kwarg, true)
        }
        "groupby" => {
            // Group items by attribute: groupby(attribute="field")
            match value.destructure_ref() {
                DestructuredRef::Array(arr) => {
                    if let Some(attr) = get_kwarg("attribute").and_then(|v| v.as_string()) {
                        // Use Vec to maintain insertion order
                        let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
                        for item in arr.iter() {
                            let key = if let DestructuredRef::Object(obj) = item.destructure_ref() {
                                obj.get(attr.as_str())
                                    .map(|v| v.render_to_string())
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            // Find or create group
                            if let Some((_, items)) = groups.iter_mut().find(|(k, _)| k == &key) {
                                items.push(item.clone());
                            } else {
                                groups.push((key, vec![item.clone()]));
                            }
                        }
                        // Return as array of [key, items] pairs for tuple unpacking
                        let pairs: Vec<Value> = groups
                            .into_iter()
                            .map(|(k, v)| {
                                let items_arr: Value = VArray::from_iter(v).into();
                                let pair: Value =
                                    VArray::from_iter([Value::from(k.as_str()), items_arr]).into();
                                pair
                            })
                            .collect();
                        VArray::from_iter(pairs).into()
                    } else {
                        value
                    }
                }
                _ => value,
            }
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
