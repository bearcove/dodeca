//! Template renderer
//!
//! Renders templates to strings using a context.
//! This is the main public API for the template engine.

use super::ast::{self, Expr, Node, Target};
use super::error::TemplateSource;
use super::eval::{Context, Evaluator, Value};
use super::lazy::LazyValue;
use super::parser::Parser;
use camino::{Utf8Path, Utf8PathBuf};
use facet_value::{DestructuredRef, VObject, VString};
use futures::future::BoxFuture;
use miette::Result;
use std::collections::HashMap;

/// Loop control flow signals
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControl {
    /// Normal execution, continue to next node
    None,
    /// Skip to next loop iteration
    Continue,
    /// Exit the loop entirely
    Break,
}

/// A stored macro definition
#[derive(Debug, Clone)]
struct MacroDef {
    params: Vec<ast::MacroParam>,
    body: Vec<Node>,
}

/// A block override with its source template (for error reporting)
#[derive(Debug, Clone)]
struct BlockDef {
    nodes: Vec<Node>,
    source: TemplateSource,
}

/// Stored macros organized by namespace
/// Key is namespace ("self" for current template, or import alias)
/// Value is map of macro_name -> MacroDef
type MacroRegistry = HashMap<String, HashMap<String, MacroDef>>;

/// Trait for loading templates by name (for inheritance and includes)
///
/// This trait is async to support loading templates from remote sources
/// (e.g., via RPC in the cell architecture).
pub trait TemplateLoader: Send + Sync {
    /// Load a template by path/name, returning the source code
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>>;
}

/// A null loader that never finds any templates.
/// Used when rendering a template without inheritance/include support.
#[derive(Default, Clone, Copy)]
pub struct NullLoader;

impl TemplateLoader for NullLoader {
    fn load(&self, _name: &str) -> BoxFuture<'_, Option<String>> {
        Box::pin(async { None })
    }
}

/// A simple in-memory template loader
#[derive(Default)]
pub struct InMemoryLoader {
    templates: HashMap<String, String>,
}

impl InMemoryLoader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: impl Into<String>, source: impl Into<String>) {
        self.templates.insert(name.into(), source.into());
    }
}

impl TemplateLoader for InMemoryLoader {
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>> {
        let result = self.templates.get(name).cloned();
        Box::pin(async move { result })
    }
}

/// A file-based template loader that reads from a directory
pub struct FileLoader {
    root: Utf8PathBuf,
}

impl FileLoader {
    /// Create a new file loader rooted at the given directory
    pub fn new(root: impl AsRef<Utf8Path>) -> Self {
        Self {
            root: root.as_ref().to_owned(),
        }
    }

    /// Get the root directory
    pub fn root(&self) -> &Utf8Path {
        &self.root
    }
}

impl TemplateLoader for FileLoader {
    fn load(&self, name: &str) -> BoxFuture<'_, Option<String>> {
        let path = self.root.join(name);
        Box::pin(async move { std::fs::read_to_string(&path).ok() })
    }
}

/// A compiled template ready for rendering
#[derive(Debug, Clone)]
pub struct Template {
    ast: ast::Template,
    source: TemplateSource,
}

impl Template {
    /// Parse a template from source
    pub fn parse(name: impl Into<String>, source: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let source_str: String = source.into();
        let template_source = TemplateSource::new(&name, &source_str);

        let parser = Parser::new(name, source_str);
        let ast = parser.parse()?;

        Ok(Self {
            ast,
            source: template_source,
        })
    }

    /// Get the extends path if this template extends another
    pub fn extends_path(&self) -> Option<&str> {
        for node in &self.ast.body {
            match node {
                Node::Extends(e) => return Some(&e.path.value),
                Node::Text(t) if t.text.trim().is_empty() => continue,
                _ => return None,
            }
        }
        None
    }

    /// Extract block definitions from this template
    pub fn blocks(&self) -> HashMap<String, &[Node]> {
        let mut blocks = HashMap::new();
        self.collect_blocks(&self.ast.body, &mut blocks);
        blocks
    }

    fn collect_blocks<'a>(&'a self, nodes: &'a [Node], blocks: &mut HashMap<String, &'a [Node]>) {
        for node in nodes {
            if let Node::Block(block) = node {
                blocks.insert(block.name.name.clone(), &block.body);
            }
        }
    }

    /// Extract import statements from this template
    pub fn imports(&self) -> Vec<(&str, &str)> {
        self.ast
            .body
            .iter()
            .filter_map(|node| {
                if let Node::Import(import) = node {
                    Some((import.path.value.as_str(), import.alias.name.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Render the template with the given context (no inheritance support)
    pub async fn render(&self, ctx: &Context) -> Result<String> {
        let mut output = String::new();
        let null_loader = NullLoader;
        let mut renderer = Renderer {
            ctx: ctx.clone(),
            source: self.source.clone(),
            output: &mut output,
            blocks: HashMap::new(),
            loader: Some(&null_loader),
            macros: HashMap::new(),
        };
        renderer.collect_macros(&self.ast.body);
        let _ = renderer.render_nodes(&self.ast.body).await?;
        Ok(output)
    }

    /// Render the template with a simple key-value context
    pub async fn render_with<I, K, V>(&self, vars: I) -> Result<String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        let mut ctx = Context::new();
        for (k, v) in vars {
            ctx.set(k, v.into());
        }
        self.render(&ctx).await
    }
}

/// Template engine with support for inheritance and includes
pub struct Engine<L: TemplateLoader> {
    loader: L,
}

impl<L: TemplateLoader> Engine<L> {
    /// Create a new engine with the given loader
    pub fn new(loader: L) -> Self {
        Self { loader }
    }

    /// Load a template (no caching - that's picante's job)
    pub async fn load(&self, name: &str) -> Result<Template> {
        let source = self
            .loader
            .load(name)
            .await
            .ok_or_else(|| miette::miette!("Template not found: {}", name))?;
        Template::parse(name, source)
    }

    /// Render a template by name with inheritance support
    pub async fn render(&mut self, name: &str, ctx: &Context) -> Result<String> {
        // Load the template
        let template = self.load(name).await?;

        // Check for extends
        if let Some(parent_path) = template.extends_path() {
            // Collect blocks from child template, along with the source for error reporting
            let child_blocks: HashMap<String, BlockDef> = template
                .blocks()
                .into_iter()
                .map(|(name, nodes)| {
                    (
                        name,
                        BlockDef {
                            nodes: nodes.to_vec(),
                            source: template.source.clone(),
                        },
                    )
                })
                .collect();
            let child_imports: Vec<(String, String)> = template
                .imports()
                .into_iter()
                .map(|(p, a)| (p.to_string(), a.to_string()))
                .collect();

            // Recursively resolve the inheritance chain
            self.render_with_blocks(parent_path, ctx, child_blocks, child_imports)
                .await
        } else {
            // No inheritance, render directly
            let context_vars = ctx.variable_names();
            tracing::debug!(
                template = %name,
                context_vars = ?context_vars,
                "Engine::render: rendering template"
            );
            let mut output = String::new();
            let mut renderer = Renderer {
                ctx: ctx.clone(),
                source: template.source.clone(),
                output: &mut output,
                blocks: HashMap::new(),
                loader: Some(&self.loader),
                macros: HashMap::new(),
            };
            renderer.collect_macros(&template.ast.body);
            let _ = renderer.render_nodes(&template.ast.body).await?;
            Ok(output)
        }
    }

    fn render_with_blocks<'a>(
        &'a mut self,
        name: &'a str,
        ctx: &'a Context,
        child_blocks: HashMap<String, BlockDef>,
        child_imports: Vec<(String, String)>,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let template = self.load(name).await?;

            // Check if parent also extends
            if let Some(grandparent_path) = template.extends_path() {
                // Merge blocks: child blocks override parent blocks
                let parent_blocks = template.blocks();
                let mut merged_blocks: HashMap<String, BlockDef> = HashMap::new();

                // Add parent blocks first (with this template's source)
                for (name, nodes) in parent_blocks {
                    merged_blocks.insert(
                        name,
                        BlockDef {
                            nodes: nodes.to_vec(),
                            source: template.source.clone(),
                        },
                    );
                }
                // Child blocks override (keeping their original source)
                for (name, block_def) in child_blocks {
                    merged_blocks.insert(name, block_def);
                }

                // Merge imports: add parent imports first, then child imports
                let parent_imports: Vec<(String, String)> = template
                    .imports()
                    .into_iter()
                    .map(|(p, a)| (p.to_string(), a.to_string()))
                    .collect();
                let mut all_imports = parent_imports;
                all_imports.extend(child_imports);

                self.render_with_blocks(grandparent_path, ctx, merged_blocks, all_imports)
                    .await
            } else {
                // This is the root template, render it with block overrides
                let mut output = String::new();
                let mut renderer = Renderer {
                    ctx: ctx.clone(),
                    source: template.source.clone(),
                    output: &mut output,
                    blocks: child_blocks,
                    loader: Some(&self.loader),
                    macros: HashMap::new(),
                };

                // Process imports from child templates
                for (path, alias) in &child_imports {
                    renderer.load_macros_from(path, alias).await?;
                }

                renderer.collect_macros(&template.ast.body);
                let _ = renderer.render_nodes(&template.ast.body).await?;
                Ok(output)
            }
        })
    }
}

/// Internal renderer state
struct Renderer<'a, L: TemplateLoader> {
    ctx: Context,
    /// Current source for error reporting (may be swapped when rendering child blocks)
    source: TemplateSource,
    output: &'a mut String,
    /// Block overrides from child templates (with their source for error reporting)
    blocks: HashMap<String, BlockDef>,
    /// Template loader for includes and imports
    loader: Option<&'a L>,
    /// Macro definitions by namespace
    macros: MacroRegistry,
}

impl<'a, L: TemplateLoader> Renderer<'a, L> {
    fn render_nodes<'b>(&'b mut self, nodes: &'b [Node]) -> BoxFuture<'b, Result<LoopControl>> {
        Box::pin(async move {
            for node in nodes {
                let control = self.render_node(node).await?;
                if control != LoopControl::None {
                    return Ok(control);
                }
            }
            Ok(LoopControl::None)
        })
    }

    /// Render nodes with a different source (for block overrides from child templates)
    fn render_nodes_with_source<'b>(
        &'b mut self,
        nodes: &'b [Node],
        source: TemplateSource,
    ) -> BoxFuture<'b, Result<LoopControl>> {
        Box::pin(async move {
            let original_source = std::mem::replace(&mut self.source, source);
            let result = self.render_nodes(nodes).await;
            self.source = original_source;
            result
        })
    }

    fn render_node<'b>(&'b mut self, node: &'b Node) -> BoxFuture<'b, Result<LoopControl>> {
        Box::pin(async move {
            match node {
                Node::Text(text) => {
                    self.output.push_str(&text.text);
                }
                Node::Print(print) => {
                    // Check if this is a macro call
                    if let Expr::MacroCall(macro_call) = &print.expr {
                        // Evaluate arguments (macros need concrete values)
                        let eval = Evaluator::new(&self.ctx, &self.source);
                        let mut args = Vec::with_capacity(macro_call.args.len());
                        for a in &macro_call.args {
                            args.push(eval.eval_concrete(a).await?);
                        }
                        let mut kwargs = Vec::with_capacity(macro_call.kwargs.len());
                        for (ident, expr) in &macro_call.kwargs {
                            kwargs.push((ident.name.clone(), eval.eval_concrete(expr).await?));
                        }

                        // Call the macro
                        let result = self
                            .call_macro(
                                &macro_call.namespace.name,
                                &macro_call.macro_name.name,
                                &args,
                                &kwargs,
                            )
                            .await?;
                        // Macro output is already HTML, don't escape it
                        self.output.push_str(&result);
                    } else {
                        let eval = Evaluator::new(&self.ctx, &self.source);
                        let value = eval.eval(&print.expr).await?;
                        // Skip escaping for safe values, auto-escape everything else
                        let s = if value.is_safe().await {
                            value.render_to_string().await
                        } else {
                            html_escape(&value.render_to_string().await)
                        };
                        self.output.push_str(&s);
                    }
                }
                Node::If(if_node) => {
                    let eval = Evaluator::new(&self.ctx, &self.source);
                    let condition = eval.eval(&if_node.condition).await?;

                    if condition.is_truthy().await {
                        let control = self.render_nodes(&if_node.then_body).await?;
                        if control != LoopControl::None {
                            return Ok(control);
                        }
                    } else {
                        // Check elif branches
                        let mut handled = false;
                        for elif in &if_node.elif_branches {
                            let eval = Evaluator::new(&self.ctx, &self.source);
                            let cond = eval.eval(&elif.condition).await?;
                            if cond.is_truthy().await {
                                let control = self.render_nodes(&elif.body).await?;
                                if control != LoopControl::None {
                                    return Ok(control);
                                }
                                handled = true;
                                break;
                            }
                        }

                        // Else branch
                        if !handled && let Some(else_body) = &if_node.else_body {
                            let control = self.render_nodes(else_body).await?;
                            if control != LoopControl::None {
                                return Ok(control);
                            }
                        }
                    }
                }
                Node::For(for_node) => {
                    let eval = Evaluator::new(&self.ctx, &self.source);
                    let iter_value = eval.eval(&for_node.iter).await?;

                    // Use LazyValue's iteration which handles lazy/concrete uniformly
                    let items: Vec<LazyValue> = iter_value.iter_values().await;

                    if items.is_empty() {
                        // Render else body if present
                        if let Some(else_body) = &for_node.else_body {
                            let control = self.render_nodes(else_body).await?;
                            if control != LoopControl::None {
                                return Ok(control);
                            }
                        }
                    } else {
                        let len = items.len();
                        'for_loop: for (index, item) in items.into_iter().enumerate() {
                            self.ctx.push_scope();

                            // Bind loop variable(s)
                            match &for_node.target {
                                Target::Single { name, .. } => {
                                    self.ctx.set(name.clone(), item);
                                }
                                Target::Tuple { names, .. } => {
                                    // For tuple unpacking, resolve to get the concrete value
                                    let resolved = item.try_resolve().await.unwrap_or(Value::NULL);
                                    match resolved.destructure_ref() {
                                        DestructuredRef::Array(parts) => {
                                            for (i, (name, _)) in names.iter().enumerate() {
                                                let val =
                                                    parts.get(i).cloned().unwrap_or(Value::NULL);
                                                self.ctx.set(name.clone(), val);
                                            }
                                        }
                                        DestructuredRef::Object(obj) => {
                                            // Special case: dict iteration gives key, value
                                            if names.len() == 2 {
                                                if let Some(key) = obj.get("key") {
                                                    self.ctx.set(names[0].0.clone(), key.clone());
                                                }
                                                if let Some(value) = obj.get("value") {
                                                    self.ctx.set(names[1].0.clone(), value.clone());
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // Bind loop helper variables
                            let mut loop_var = VObject::new();
                            loop_var
                                .insert(VString::from("index"), Value::from((index + 1) as i64));
                            loop_var.insert(VString::from("index0"), Value::from(index as i64));
                            loop_var.insert(VString::from("first"), Value::from(index == 0));
                            loop_var.insert(VString::from("last"), Value::from(index == len - 1));
                            loop_var.insert(VString::from("length"), Value::from(len as i64));
                            self.ctx.set("loop", Value::from(loop_var));

                            let control = self.render_nodes(&for_node.body).await?;
                            self.ctx.pop_scope();

                            match control {
                                LoopControl::None => {}
                                LoopControl::Continue => continue 'for_loop,
                                LoopControl::Break => break 'for_loop,
                            }
                        }
                    }
                }
                Node::Include(_include) => {
                    // TODO: Template loading/caching
                    self.output.push_str("<!-- include not implemented -->");
                }
                Node::Block(block) => {
                    // Check if we have an override for this block
                    let control =
                        if let Some(block_def) = self.blocks.get(&block.name.name).cloned() {
                            // Render the overridden block content with its original source
                            // (for correct error reporting)
                            self.render_nodes_with_source(&block_def.nodes, block_def.source)
                                .await?
                        } else {
                            // Render the default block content
                            self.render_nodes(&block.body).await?
                        };
                    if control != LoopControl::None {
                        return Ok(control);
                    }
                }
                Node::Extends(_extends) => {
                    // Extends is handled at the Engine level, not during node rendering
                    // When we reach here, we're rendering the parent template
                }
                Node::Comment(_) => {
                    // Comments are not rendered
                }
                Node::Set(set_node) => {
                    let eval = Evaluator::new(&self.ctx, &self.source);
                    let value = eval.eval(&set_node.value).await?;
                    self.ctx.set(set_node.name.name.clone(), value);
                }
                Node::Import(import) => {
                    // Load macros from the imported template
                    let loader = &self.loader;
                    if loader.is_none() {
                        tracing::warn!(
                            import_path = %import.path.value,
                            "Import: no loader available"
                        );
                    }
                    if let Some(loader) = loader
                        && let Some(source) = loader.load(&import.path.value).await
                        && let Ok(template) = Template::parse(&import.path.value, source)
                    {
                        // Extract macros from the imported template
                        let mut namespace_macros = HashMap::new();
                        for node in &template.ast.body {
                            if let Node::Macro(m) = node {
                                namespace_macros.insert(
                                    m.name.name.clone(),
                                    MacroDef {
                                        params: m.params.clone(),
                                        body: m.body.clone(),
                                    },
                                );
                            }
                        }
                        tracing::debug!(
                            import_path = %import.path.value,
                            alias = %import.alias.name,
                            macros = namespace_macros.len(),
                            "Import: loaded macros"
                        );
                        self.macros
                            .insert(import.alias.name.clone(), namespace_macros);
                    } else {
                        tracing::warn!(
                            import_path = %import.path.value,
                            "Import: failed to load template"
                        );
                    }
                }
                Node::Macro(_macro_def) => {
                    // Macro definitions are collected by collect_macros before rendering
                }
                Node::Continue(_) => {
                    return Ok(LoopControl::Continue);
                }
                Node::Break(_) => {
                    return Ok(LoopControl::Break);
                }
            }

            Ok(LoopControl::None)
        })
    }

    /// Collect macro definitions from the template body into the "self" namespace
    fn collect_macros(&mut self, nodes: &[Node]) {
        let mut self_macros = HashMap::new();
        for node in nodes {
            if let Node::Macro(m) = node {
                self_macros.insert(
                    m.name.name.clone(),
                    MacroDef {
                        params: m.params.clone(),
                        body: m.body.clone(),
                    },
                );
            }
        }
        if !self_macros.is_empty() {
            self.macros.insert("self".to_string(), self_macros);
        }
    }

    /// Load macros from an imported template file into the given namespace
    async fn load_macros_from(&mut self, path: &str, alias: &str) -> Result<()> {
        if let Some(loader) = &self.loader
            && let Some(source) = loader.load(path).await
        {
            let template = Template::parse(path, source)?;
            // Extract macros from the imported template
            let mut namespace_macros = HashMap::new();
            for node in &template.ast.body {
                if let Node::Macro(m) = node {
                    namespace_macros.insert(
                        m.name.name.clone(),
                        MacroDef {
                            params: m.params.clone(),
                            body: m.body.clone(),
                        },
                    );
                }
            }
            self.macros.insert(alias.to_string(), namespace_macros);
        }
        Ok(())
    }

    /// Call a macro and return its rendered output
    fn call_macro<'b>(
        &'b mut self,
        namespace: &'b str,
        macro_name: &'b str,
        args: &'b [Value],
        kwargs: &'b [(String, Value)],
    ) -> BoxFuture<'b, Result<String>> {
        Box::pin(async move {
            // Find the macro
            let macro_def = self
                .macros
                .get(namespace)
                .and_then(|ns| ns.get(macro_name))
                .cloned()
                .ok_or_else(|| miette::miette!("Macro not found: {}::{}", namespace, macro_name))?;

            // Save output and create new buffer for macro
            let mut macro_output = String::new();
            std::mem::swap(self.output, &mut macro_output);

            // Set up "self" namespace with macros from the called macro's namespace
            // This allows macros to call other macros from the same file via self::
            // Only do this if we're not already calling from the "self" namespace
            let saved_self = if namespace != "self" {
                let saved = self.macros.remove("self");
                if let Some(ns_macros) = self.macros.get(namespace).cloned() {
                    self.macros.insert("self".to_string(), ns_macros);
                }
                saved
            } else {
                None
            };

            // Push a new scope for macro arguments
            self.ctx.push_scope();

            // Bind positional arguments
            for (i, param) in macro_def.params.iter().enumerate() {
                let value: LazyValue = if i < args.len() {
                    LazyValue::concrete(args[i].clone())
                } else if let Some((_, v)) = kwargs.iter().find(|(k, _)| k == &param.name.name) {
                    LazyValue::concrete(v.clone())
                } else if let Some(ref default_expr) = param.default {
                    // Evaluate default value
                    let eval = Evaluator::new(&self.ctx, &self.source);
                    eval.eval(default_expr).await?
                } else {
                    LazyValue::concrete(Value::NULL)
                };
                self.ctx.set(param.name.name.clone(), value);
            }

            // Render macro body (ignore loop control - macros don't propagate continue/break)
            let _ = self.render_nodes(&macro_def.body).await?;

            // Restore scope
            self.ctx.pop_scope();

            // Restore "self" namespace (only if we modified it)
            if namespace != "self" {
                self.macros.remove("self");
                if let Some(saved) = saved_self {
                    self.macros.insert("self".to_string(), saved);
                }
            }

            // Swap back and return macro output
            std::mem::swap(self.output, &mut macro_output);
            Ok(macro_output)
        })
    }
}

/// HTML escape a string
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::ValueExt;
    use facet_value::VArray;

    #[tokio::test]
    async fn test_simple_text() {
        let t = Template::parse("test", "Hello, world!").unwrap();
        assert_eq!(t.render(&Context::new()).await.unwrap(), "Hello, world!");
    }

    #[tokio::test]
    async fn test_variable() {
        let t = Template::parse("test", "Hello, {{ name }}!").unwrap();
        let result = t
            .render_with([("name", Value::from("Alice"))])
            .await
            .unwrap();
        assert_eq!(result, "Hello, Alice!");
    }

    #[tokio::test]
    async fn test_if_true() {
        let t = Template::parse("test", "{% if show %}visible{% endif %}").unwrap();
        let result = t.render_with([("show", Value::from(true))]).await.unwrap();
        assert_eq!(result, "visible");
    }

    #[tokio::test]
    async fn test_if_false() {
        let t = Template::parse("test", "{% if show %}visible{% endif %}").unwrap();
        let result = t.render_with([("show", Value::from(false))]).await.unwrap();
        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_if_else() {
        let t = Template::parse("test", "{% if show %}yes{% else %}no{% endif %}").unwrap();
        let result = t.render_with([("show", Value::from(false))]).await.unwrap();
        assert_eq!(result, "no");
    }

    #[tokio::test]
    async fn test_for_loop() {
        let t = Template::parse("test", "{% for item in items %}{{ item }} {% endfor %}").unwrap();
        let items: Value =
            VArray::from_iter([Value::from("a"), Value::from("b"), Value::from("c")]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "a b c ");
    }

    #[tokio::test]
    async fn test_loop_index() {
        let t =
            Template::parse("test", "{% for x in items %}{{ loop.index }}{% endfor %}").unwrap();
        let items: Value = VArray::from_iter([Value::from("a"), Value::from("b")]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "12");
    }

    #[tokio::test]
    async fn test_filter() {
        let t = Template::parse("test", "{{ name | upper }}").unwrap();
        let result = t
            .render_with([("name", Value::from("alice"))])
            .await
            .unwrap();
        assert_eq!(result, "ALICE");
    }

    #[tokio::test]
    async fn test_field_access() {
        let t = Template::parse("test", "{{ user.name }}").unwrap();
        let mut user = VObject::new();
        user.insert(VString::from("name"), Value::from("Bob"));
        let user_val: Value = user.into();
        let result = t.render_with([("user", user_val)]).await.unwrap();
        assert_eq!(result, "Bob");
    }

    #[tokio::test]
    async fn test_html_escape() {
        let t = Template::parse("test", "{{ content }}").unwrap();
        let result = t
            .render_with([("content", Value::from("<script>alert('xss')</script>"))])
            .await
            .unwrap();
        assert_eq!(
            result,
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"
        );
    }

    #[tokio::test]
    async fn test_safe_filter() {
        let t = Template::parse("test", "{{ content | safe }}").unwrap();
        let result = t
            .render_with([("content", Value::from("<b>bold</b>"))])
            .await
            .unwrap();
        // Note: safe filter doesn't work with facet_value since we can't track "safe" status
        // This test just verifies the filter doesn't error
        assert!(result.contains("bold"));
    }

    #[tokio::test]
    async fn test_safe_filter_with_other_filters() {
        let t = Template::parse("test", "{{ content | upper | safe }}").unwrap();
        let result = t
            .render_with([("content", Value::from("<b>bold</b>"))])
            .await
            .unwrap();
        assert!(result.contains("BOLD"));
    }

    #[tokio::test]
    async fn test_split_filter() {
        let t = Template::parse(
            "test",
            "{% for p in path | split(pat=\"/\") %}[{{ p }}]{% endfor %}",
        )
        .unwrap();
        let result = t
            .render_with([("path", Value::from("a/b/c"))])
            .await
            .unwrap();
        assert_eq!(result, "[a][b][c]");
    }

    #[tokio::test]
    async fn test_global_function() {
        let t = Template::parse("test", "{{ greet(name) }}").unwrap();
        let mut ctx = Context::new();
        ctx.set("name", Value::from("World"));
        ctx.register_fn(
            "greet",
            Box::new(|args: &[Value], _kwargs| {
                let name = args
                    .first()
                    .map(|v| v.render_to_string())
                    .unwrap_or_default();
                Box::pin(async move { Ok(Value::from(format!("Hello, {name}!").as_str())) })
            }),
        );
        let result = t.render(&ctx).await.unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_set() {
        let t = Template::parse("test", "{% set x = 42 %}{{ x }}").unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "42");
    }

    #[tokio::test]
    async fn test_set_expression() {
        let t = Template::parse("test", "{% set x = 2 + 3 %}{{ x }}").unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "5");
    }

    #[tokio::test]
    async fn test_macro_simple() {
        let t = Template::parse(
            "test",
            "{% macro greet(name) %}Hello, {{ name }}!{% endmacro %}{{ self::greet(\"World\") }}",
        )
        .unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_macro_with_default() {
        let t = Template::parse(
            "test",
            "{% macro greet(name=\"Guest\") %}Hello, {{ name }}!{% endmacro %}{{ self::greet() }}",
        )
        .unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "Hello, Guest!");
    }

    #[tokio::test]
    async fn test_macro_override_default() {
        let t = Template::parse("test", "{% macro greet(name=\"Guest\") %}Hello, {{ name }}!{% endmacro %}{{ self::greet(\"Alice\") }}").unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "Hello, Alice!");
    }

    #[tokio::test]
    async fn test_macro_kwargs() {
        let t = Template::parse("test", "{% macro greet(name) %}Hello, {{ name }}!{% endmacro %}{{ self::greet(name=\"Bob\") }}").unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "Hello, Bob!");
    }

    #[tokio::test]
    async fn test_macro_multiple_calls() {
        let t = Template::parse(
            "test",
            "{% macro twice(x) %}{{ x }}{{ x }}{% endmacro %}{{ self::twice(\"ab\") }}",
        )
        .unwrap();
        let result = t.render(&Context::new()).await.unwrap();
        assert_eq!(result, "abab");
    }

    #[tokio::test]
    async fn test_template_inheritance() {
        let mut loader = InMemoryLoader::new();
        loader.add(
            "base.html",
            "Header {% block content %}default{% endblock %} Footer",
        );
        loader.add(
            "child.html",
            "{% extends \"base.html\" %}{% block content %}CUSTOM{% endblock %}",
        );

        let mut engine = Engine::new(loader);
        let result = engine.render("child.html", &Context::new()).await.unwrap();
        assert_eq!(result, "Header CUSTOM Footer");
    }

    #[tokio::test]
    async fn test_template_inheritance_with_variables() {
        let mut loader = InMemoryLoader::new();
        loader.add("base.html", "{% block title %}Default{% endblock %}");
        loader.add(
            "child.html",
            "{% extends \"base.html\" %}{% block title %}{{ page_title }}{% endblock %}",
        );

        let mut engine = Engine::new(loader);
        let mut ctx = Context::new();
        ctx.set("page_title", Value::from("My Page"));
        let result = engine.render("child.html", &ctx).await.unwrap();
        assert_eq!(result, "My Page");
    }

    #[tokio::test]
    async fn test_template_no_extends() {
        let mut loader = InMemoryLoader::new();
        loader.add("simple.html", "Just {{ content }}");

        let mut engine = Engine::new(loader);
        let mut ctx = Context::new();
        ctx.set("content", Value::from("text"));
        let result = engine.render("simple.html", &ctx).await.unwrap();
        assert_eq!(result, "Just text");
    }

    #[tokio::test]
    async fn test_block_default_content() {
        let mut loader = InMemoryLoader::new();
        loader.add("base.html", "{% block main %}DEFAULT{% endblock %}");
        loader.add("child.html", "{% extends \"base.html\" %}");

        let mut engine = Engine::new(loader);
        let result = engine.render("child.html", &Context::new()).await.unwrap();
        assert_eq!(result, "DEFAULT");
    }

    #[tokio::test]
    async fn test_multiple_blocks() {
        let mut loader = InMemoryLoader::new();
        loader.add(
            "base.html",
            "[{% block a %}A{% endblock %}][{% block b %}B{% endblock %}]",
        );
        loader.add(
            "child.html",
            "{% extends \"base.html\" %}{% block a %}X{% endblock %}",
        );

        let mut engine = Engine::new(loader);
        let result = engine.render("child.html", &Context::new()).await.unwrap();
        assert_eq!(result, "[X][B]");
    }

    #[tokio::test]
    async fn test_macro_import() {
        let mut loader = InMemoryLoader::new();
        loader.add(
            "macros.html",
            "{% macro hello(name) %}Hello, {{ name }}!{% endmacro %}",
        );
        loader.add(
            "page.html",
            "{% import \"macros.html\" as m %}{{ m::hello(\"World\") }}",
        );

        let mut engine = Engine::new(loader);
        let result = engine.render("page.html", &Context::new()).await.unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_is_starting_with() {
        let t = Template::parse(
            "test",
            "{% if path is starting_with(\"/admin\") %}admin{% else %}user{% endif %}",
        )
        .unwrap();
        let result = t
            .render_with([("path", Value::from("/admin/dashboard"))])
            .await
            .unwrap();
        assert_eq!(result, "admin");
    }

    #[tokio::test]
    async fn test_is_containing() {
        let t = Template::parse(
            "test",
            "{% if text is containing(\"foo\") %}yes{% else %}no{% endif %}",
        )
        .unwrap();
        let result = t
            .render_with([("text", Value::from("hello foo bar"))])
            .await
            .unwrap();
        assert_eq!(result, "yes");
    }

    #[tokio::test]
    async fn test_is_not() {
        // Note: "none" is a keyword, so we test with "defined" instead
        let t = Template::parse("test", "{% if x is not undefined %}has_value{% endif %}").unwrap();
        let result = t.render_with([("x", Value::from(42i64))]).await.unwrap();
        assert_eq!(result, "has_value");
    }

    #[tokio::test]
    async fn test_continue_in_loop() {
        let t = Template::parse(
            "test",
            "{% for i in items %}{% if i == 2 %}{% continue %}{% endif %}{{ i }}{% endfor %}",
        )
        .unwrap();
        let items: Value =
            VArray::from_iter([Value::from(1i64), Value::from(2i64), Value::from(3i64)]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "13");
    }

    #[tokio::test]
    async fn test_break_in_loop() {
        let t = Template::parse(
            "test",
            "{% for i in items %}{% if i == 2 %}{% break %}{% endif %}{{ i }}{% endfor %}",
        )
        .unwrap();
        let items: Value =
            VArray::from_iter([Value::from(1i64), Value::from(2i64), Value::from(3i64)]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "1");
    }

    #[tokio::test]
    async fn test_continue_with_loop_index() {
        let t = Template::parse("test", "{% for x in items %}{% if loop.index == 2 %}{% continue %}{% endif %}[{{ x }}]{% endfor %}").unwrap();
        let items: Value =
            VArray::from_iter([Value::from("a"), Value::from("b"), Value::from("c")]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "[a][c]");
    }

    #[tokio::test]
    async fn test_break_in_nested_if() {
        let t = Template::parse("test", "{% for i in items %}{% if i > 1 %}{% if i == 2 %}{% break %}{% endif %}{% endif %}{{ i }}{% endfor %}").unwrap();
        let items: Value =
            VArray::from_iter([Value::from(1i64), Value::from(2i64), Value::from(3i64)]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "1");
    }

    #[tokio::test]
    async fn test_error_source_in_inherited_block() {
        let mut loader = InMemoryLoader::new();
        loader.add("base.html", "{% block content %}{% endblock %}");
        loader.add(
            "child.html",
            "{% extends \"base.html\" %}{% block content %}{{ undefined_var }}{% endblock %}",
        );

        let mut engine = Engine::new(loader);
        let result = engine.render("child.html", &Context::new()).await;
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        // The error should reference child.html, not base.html
        assert!(
            err.contains("child.html"),
            "Error should reference child.html: {}",
            err
        );
    }

    // ========================================================================
    // New filter tests (#71, #72, #73, #74, #78)
    // ========================================================================

    #[tokio::test]
    async fn test_typeof_filter() {
        let t = Template::parse("test", "{{ x | typeof }}").unwrap();
        assert_eq!(
            t.render_with([("x", Value::from("hello"))]).await.unwrap(),
            "string"
        );
        assert_eq!(
            t.render_with([("x", Value::from(42i64))]).await.unwrap(),
            "number"
        );
        assert_eq!(t.render_with([("x", Value::NULL)]).await.unwrap(), "none");

        let arr: Value = VArray::from_iter([Value::from(1i64)]).into();
        assert_eq!(t.render_with([("x", arr)]).await.unwrap(), "list");

        let obj: Value = VObject::new().into();
        assert_eq!(t.render_with([("x", obj)]).await.unwrap(), "dict");
    }

    #[tokio::test]
    async fn test_slice_filter_kwargs() {
        let t = Template::parse(
            "test",
            "{% for x in items | slice(end=2) %}{{ x }}{% endfor %}",
        )
        .unwrap();
        let items: Value =
            VArray::from_iter([Value::from("a"), Value::from("b"), Value::from("c")]).into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "ab");
    }

    #[tokio::test]
    async fn test_slice_filter_start_end() {
        let t = Template::parse(
            "test",
            "{% for x in items | slice(start=1, end=3) %}{{ x }}{% endfor %}",
        )
        .unwrap();
        let items: Value = VArray::from_iter([
            Value::from("a"),
            Value::from("b"),
            Value::from("c"),
            Value::from("d"),
        ])
        .into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "bc");
    }

    #[tokio::test]
    async fn test_slice_filter_positional() {
        let t = Template::parse(
            "test",
            "{% for x in items | slice(1, 3) %}{{ x }}{% endfor %}",
        )
        .unwrap();
        let items: Value = VArray::from_iter([
            Value::from("a"),
            Value::from("b"),
            Value::from("c"),
            Value::from("d"),
        ])
        .into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "bc");
    }

    #[tokio::test]
    async fn test_slice_filter_empty() {
        let t = Template::parse(
            "test",
            "{% for x in items | slice(end=0) %}{{ x }}{% endfor %}",
        )
        .unwrap();
        let items: Value = VArray::from_iter([Value::from("a"), Value::from("b")]).into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "");
    }

    #[tokio::test]
    async fn test_map_filter() {
        let t = Template::parse(
            "test",
            "{{ items | map(attribute=\"name\") | join(\", \") }}",
        )
        .unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Alice"));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Bob"));

        let items: Value = VArray::from_iter([Value::from(item1), Value::from(item2)]).into();
        assert_eq!(
            t.render_with([("items", items)]).await.unwrap(),
            "Alice, Bob"
        );
    }

    #[tokio::test]
    async fn test_map_filter_missing_attr() {
        let t =
            Template::parse("test", "{{ items | map(attribute=\"missing\") | length }}").unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Alice"));

        let items: Value = VArray::from_iter([Value::from(item1)]).into();
        // Items without the attribute are filtered out
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "0");
    }

    #[tokio::test]
    async fn test_selectattr_truthy() {
        let t = Template::parse(
            "test",
            "{% for x in items | selectattr(\"active\") %}{{ x.name }}{% endfor %}",
        )
        .unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Alice"));
        item1.insert(VString::from("active"), Value::from(true));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Bob"));
        item2.insert(VString::from("active"), Value::from(false));
        let mut item3 = VObject::new();
        item3.insert(VString::from("name"), Value::from("Carol"));
        item3.insert(VString::from("active"), Value::from(true));

        let items: Value =
            VArray::from_iter([Value::from(item1), Value::from(item2), Value::from(item3)]).into();
        assert_eq!(
            t.render_with([("items", items)]).await.unwrap(),
            "AliceCarol"
        );
    }

    #[tokio::test]
    async fn test_selectattr_eq() {
        let t = Template::parse("test", "{% for x in items | selectattr(\"status\", \"eq\", \"active\") %}{{ x.name }}{% endfor %}").unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Alice"));
        item1.insert(VString::from("status"), Value::from("active"));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Bob"));
        item2.insert(VString::from("status"), Value::from("inactive"));

        let items: Value = VArray::from_iter([Value::from(item1), Value::from(item2)]).into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "Alice");
    }

    #[tokio::test]
    async fn test_selectattr_gt() {
        let t = Template::parse(
            "test",
            "{% for x in items | selectattr(\"weight\", \"gt\", 5) %}{{ x.name }}{% endfor %}",
        )
        .unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Heavy"));
        item1.insert(VString::from("weight"), Value::from(10i64));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Light"));
        item2.insert(VString::from("weight"), Value::from(3i64));

        let items: Value = VArray::from_iter([Value::from(item1), Value::from(item2)]).into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "Heavy");
    }

    #[tokio::test]
    async fn test_rejectattr_truthy() {
        let t = Template::parse(
            "test",
            "{% for x in items | rejectattr(\"draft\") %}{{ x.name }}{% endfor %}",
        )
        .unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Published"));
        item1.insert(VString::from("draft"), Value::from(false));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Draft"));
        item2.insert(VString::from("draft"), Value::from(true));

        let items: Value = VArray::from_iter([Value::from(item1), Value::from(item2)]).into();
        assert_eq!(
            t.render_with([("items", items)]).await.unwrap(),
            "Published"
        );
    }

    #[tokio::test]
    async fn test_selectattr_starting_with() {
        let t = Template::parse("test", "{% for x in items | selectattr(\"path\", \"starting_with\", \"/admin\") %}{{ x.name }}{% endfor %}").unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Admin"));
        item1.insert(VString::from("path"), Value::from("/admin/dashboard"));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("User"));
        item2.insert(VString::from("path"), Value::from("/user/profile"));

        let items: Value = VArray::from_iter([Value::from(item1), Value::from(item2)]).into();
        assert_eq!(t.render_with([("items", items)]).await.unwrap(), "Admin");
    }

    #[tokio::test]
    async fn test_groupby_filter() {
        // Use tuple unpacking to access category and items
        let t = Template::parse("test", "{% for category, group_items in items | groupby(attribute=\"category\") %}[{{ category }}:{% for x in group_items %}{{ x.name }}{% endfor %}]{% endfor %}").unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Apple"));
        item1.insert(VString::from("category"), Value::from("fruit"));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Carrot"));
        item2.insert(VString::from("category"), Value::from("vegetable"));
        let mut item3 = VObject::new();
        item3.insert(VString::from("name"), Value::from("Banana"));
        item3.insert(VString::from("category"), Value::from("fruit"));

        let items: Value =
            VArray::from_iter([Value::from(item1), Value::from(item2), Value::from(item3)]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        // Order is preserved: fruit first (Apple, Banana), then vegetable (Carrot)
        assert_eq!(result, "[fruit:AppleBanana][vegetable:Carrot]");
    }

    #[tokio::test]
    async fn test_groupby_tuple_unpacking() {
        // Test using tuple unpacking syntax in for loop
        let t = Template::parse("test", "{% for category, posts in items | groupby(attribute=\"cat\") %}{{ category }}:{{ posts | length }};{% endfor %}").unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("cat"), Value::from("A"));
        let mut item2 = VObject::new();
        item2.insert(VString::from("cat"), Value::from("B"));
        let mut item3 = VObject::new();
        item3.insert(VString::from("cat"), Value::from("A"));

        let items: Value =
            VArray::from_iter([Value::from(item1), Value::from(item2), Value::from(item3)]).into();
        let result = t.render_with([("items", items)]).await.unwrap();
        assert_eq!(result, "A:2;B:1;");
    }

    #[tokio::test]
    async fn test_filters_chained() {
        // Test chaining multiple new filters
        let t = Template::parse(
            "test",
            "{{ items | selectattr(\"active\") | map(attribute=\"name\") | join(\", \") }}",
        )
        .unwrap();

        let mut item1 = VObject::new();
        item1.insert(VString::from("name"), Value::from("Alice"));
        item1.insert(VString::from("active"), Value::from(true));
        let mut item2 = VObject::new();
        item2.insert(VString::from("name"), Value::from("Bob"));
        item2.insert(VString::from("active"), Value::from(false));
        let mut item3 = VObject::new();
        item3.insert(VString::from("name"), Value::from("Carol"));
        item3.insert(VString::from("active"), Value::from(true));

        let items: Value =
            VArray::from_iter([Value::from(item1), Value::from(item2), Value::from(item3)]).into();
        assert_eq!(
            t.render_with([("items", items)]).await.unwrap(),
            "Alice, Carol"
        );
    }

    #[test]
    fn test_unclosed_expression_error() {
        // Unclosed {{ should produce a parse error
        let result = Template::parse("test", "Hello {{ name");
        assert!(
            result.is_err(),
            "Unclosed expression should produce parse error"
        );
    }

    #[test]
    fn test_unclosed_expression_in_html_template() {
        // Test with content similar to the actual test case
        let template = r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title</title>
</head>
<body>
  <h1>{{ section.title</h1>
  {{ section.content | safe }}
</body>
</html>"#;
        let result = Template::parse("index.html", template);
        println!("\n\nResult: {:?}\n\n", result);
        assert!(
            result.is_err(),
            "Template with unclosed expression should produce parse error"
        );
    }
}
