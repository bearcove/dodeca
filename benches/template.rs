//! Benchmarks for template engine
//!
//! Run with: cargo bench --bench template
//!
//! Benchmarks cover:
//! - Lexing (tokenization)
//! - Parsing (AST generation)
//! - Full render (parse + evaluate)

use divan::{black_box, Bencher};
use dodeca::template::{Context, Engine, InMemoryLoader, Value};
use dodeca::template::lexer::Lexer;
use dodeca::template::parser::Parser;
use std::collections::HashMap;
use std::sync::Arc;

fn main() {
    divan::main();
}

// ============================================================================
// Template generators
// ============================================================================

/// Simple template with just text
fn simple_text() -> &'static str {
    "Hello, World! This is a simple static text template."
}

/// Template with variable interpolation
fn with_variables() -> &'static str {
    r#"Hello, {{ name }}! Welcome to {{ site_name }}.
Your account was created on {{ created_date }}.
You have {{ message_count }} unread messages."#
}

/// Template with loops
fn with_loops() -> &'static str {
    r#"<ul>
{% for item in items %}
  <li>{{ item.name }}: {{ item.price }}</li>
{% endfor %}
</ul>"#
}

/// Template with conditionals
fn with_conditionals() -> &'static str {
    r#"{% if user.is_admin %}
  <div class="admin-panel">Admin Controls</div>
{% elif user.is_moderator %}
  <div class="mod-panel">Moderator Controls</div>
{% else %}
  <div class="user-panel">User Controls</div>
{% endif %}"#
}

/// Template with filters
fn with_filters() -> &'static str {
    r#"{{ title | upper }}
{{ description | lower }}
{{ content | safe }}
{{ number | default(0) }}"#
}

/// Complex realistic template (like a blog post layout)
fn complex_template() -> &'static str {
    r#"<!DOCTYPE html>
<html>
<head>
    <title>{{ page.title }} - {{ site.name }}</title>
    <meta charset="utf-8">
</head>
<body>
    <header>
        <nav>
            {% for link in nav_links %}
            <a href="{{ link.url }}"{% if link.active %} class="active"{% endif %}>{{ link.label }}</a>
            {% endfor %}
        </nav>
    </header>

    <main>
        <article>
            <h1>{{ page.title }}</h1>
            <div class="meta">
                Published on {{ page.date }}
                {% if page.author %}by {{ page.author }}{% endif %}
            </div>
            <div class="content">
                {{ page.content | safe }}
            </div>
            {% if page.tags %}
            <div class="tags">
                {% for tag in page.tags %}
                <span class="tag">{{ tag }}</span>
                {% endfor %}
            </div>
            {% endif %}
        </article>

        {% if related_posts %}
        <aside class="related">
            <h2>Related Posts</h2>
            <ul>
            {% for post in related_posts %}
                <li><a href="{{ post.url }}">{{ post.title }}</a></li>
            {% endfor %}
            </ul>
        </aside>
        {% endif %}
    </main>

    <footer>
        <p>&copy; {{ site.year }} {{ site.name }}</p>
    </footer>
</body>
</html>"#
}

/// Generate a template with many loop iterations
fn large_loop_template(iterations: usize) -> String {
    format!(
        r#"<ul>
{{% for i in range({}) %}}
<li>Item {{{{ i }}}}: Some content here with value {{{{ i * 2 }}}}</li>
{{% endfor %}}
</ul>"#,
        iterations
    )
}

// ============================================================================
// Context builders
// ============================================================================

fn simple_context() -> Context {
    let mut ctx = Context::new();
    ctx.set("name", Value::String("Alice".into()));
    ctx.set("site_name", Value::String("My Site".into()));
    ctx.set("created_date", Value::String("2024-01-15".into()));
    ctx.set("message_count", Value::Int(42));
    ctx
}

fn loop_context() -> Context {
    let mut ctx = Context::new();
    let items: Vec<Value> = (0..10)
        .map(|i| {
            let mut item = HashMap::new();
            item.insert("name".to_string(), Value::String(format!("Item {}", i)));
            item.insert("price".to_string(), Value::Float(i as f64 * 9.99));
            Value::Dict(item)
        })
        .collect();
    ctx.set("items", Value::List(items));
    ctx
}

fn complex_context() -> Context {
    let mut ctx = Context::new();

    // Page data
    let mut page = HashMap::new();
    page.insert("title".to_string(), Value::String("My Blog Post".into()));
    page.insert("date".to_string(), Value::String("2024-03-15".into()));
    page.insert(
        "author".to_string(),
        Value::String("John Doe".into()),
    );
    page.insert(
        "content".to_string(),
        Value::String("<p>This is the post content with <strong>HTML</strong>.</p>".into()),
    );
    page.insert(
        "tags".to_string(),
        Value::List(vec![
            Value::String("rust".into()),
            Value::String("programming".into()),
            Value::String("web".into()),
        ]),
    );
    ctx.set("page", Value::Dict(page));

    // Site data
    let mut site = HashMap::new();
    site.insert("name".to_string(), Value::String("My Blog".into()));
    site.insert("year".to_string(), Value::Int(2024));
    ctx.set("site", Value::Dict(site));

    // Navigation
    let nav_links: Vec<Value> = vec![
        {
            let mut link = HashMap::new();
            link.insert("url".to_string(), Value::String("/".into()));
            link.insert("label".to_string(), Value::String("Home".into()));
            link.insert("active".to_string(), Value::Bool(false));
            Value::Dict(link)
        },
        {
            let mut link = HashMap::new();
            link.insert("url".to_string(), Value::String("/blog".into()));
            link.insert("label".to_string(), Value::String("Blog".into()));
            link.insert("active".to_string(), Value::Bool(true));
            Value::Dict(link)
        },
        {
            let mut link = HashMap::new();
            link.insert("url".to_string(), Value::String("/about".into()));
            link.insert("label".to_string(), Value::String("About".into()));
            link.insert("active".to_string(), Value::Bool(false));
            Value::Dict(link)
        },
    ];
    ctx.set("nav_links", Value::List(nav_links));

    // Related posts
    let related_posts: Vec<Value> = (0..3)
        .map(|i| {
            let mut post = HashMap::new();
            post.insert(
                "url".to_string(),
                Value::String(format!("/posts/related-{}", i)),
            );
            post.insert(
                "title".to_string(),
                Value::String(format!("Related Post {}", i + 1)),
            );
            Value::Dict(post)
        })
        .collect();
    ctx.set("related_posts", Value::List(related_posts));

    ctx
}

// ============================================================================
// Lexer benchmarks
// ============================================================================

#[divan::bench]
fn lex_simple(bencher: Bencher) {
    let source = simple_text();
    bencher.bench(|| {
        let lexer = Lexer::new(Arc::new(black_box(source).to_string()));
        for _ in lexer {}
    });
}

#[divan::bench]
fn lex_with_variables(bencher: Bencher) {
    let source = with_variables();
    bencher.bench(|| {
        let lexer = Lexer::new(Arc::new(black_box(source).to_string()));
        for _ in lexer {}
    });
}

#[divan::bench]
fn lex_complex(bencher: Bencher) {
    let source = complex_template();
    bencher.bench(|| {
        let lexer = Lexer::new(Arc::new(black_box(source).to_string()));
        for _ in lexer {}
    });
}

// ============================================================================
// Parser benchmarks
// ============================================================================

#[divan::bench]
fn parse_simple(bencher: Bencher) {
    let source = simple_text();
    bencher.bench(|| {
        let parser = Parser::new("bench", black_box(source));
        black_box(parser.parse())
    });
}

#[divan::bench]
fn parse_with_variables(bencher: Bencher) {
    let source = with_variables();
    bencher.bench(|| {
        let parser = Parser::new("bench", black_box(source));
        black_box(parser.parse())
    });
}

#[divan::bench]
fn parse_with_loops(bencher: Bencher) {
    let source = with_loops();
    bencher.bench(|| {
        let parser = Parser::new("bench", black_box(source));
        black_box(parser.parse())
    });
}

#[divan::bench]
fn parse_with_conditionals(bencher: Bencher) {
    let source = with_conditionals();
    bencher.bench(|| {
        let parser = Parser::new("bench", black_box(source));
        black_box(parser.parse())
    });
}

#[divan::bench]
fn parse_complex(bencher: Bencher) {
    let source = complex_template();
    bencher.bench(|| {
        let parser = Parser::new("bench", black_box(source));
        black_box(parser.parse())
    });
}

// ============================================================================
// Full render benchmarks
// ============================================================================

#[divan::bench]
fn render_simple(bencher: Bencher) {
    let source = simple_text();
    let ctx = Context::new();

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", source);
        let mut engine = Engine::new(loader);
        black_box(engine.render("bench", &ctx))
    });
}

#[divan::bench]
fn render_with_variables(bencher: Bencher) {
    let source = with_variables();
    let ctx = simple_context();

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", source);
        let mut engine = Engine::new(loader);
        black_box(engine.render("bench", &ctx))
    });
}

#[divan::bench]
fn render_with_loops(bencher: Bencher) {
    let source = with_loops();
    let ctx = loop_context();

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", source);
        let mut engine = Engine::new(loader);
        black_box(engine.render("bench", &ctx))
    });
}

#[divan::bench]
fn render_complex(bencher: Bencher) {
    let source = complex_template();
    let ctx = complex_context();

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", source);
        let mut engine = Engine::new(loader);
        black_box(engine.render("bench", &ctx))
    });
}

// ============================================================================
// Scaling benchmarks
// ============================================================================

#[divan::bench(args = [10, 100, 1000])]
fn render_loop_scaling(bencher: Bencher, iterations: usize) {
    let source = large_loop_template(iterations);

    // Create context with range function
    let mut ctx = Context::new();
    ctx.register_fn(
        "range",
        Box::new(move |args: &[Value], _kwargs: &[(String, Value)]| {
            let n = match args.first() {
                Some(Value::Int(n)) => *n as usize,
                _ => 0,
            };
            Ok(Value::List((0..n).map(|i| Value::Int(i as i64)).collect()))
        }),
    );

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", &source);
        let mut engine = Engine::new(loader);
        black_box(engine.render("bench", &ctx))
    });
}
