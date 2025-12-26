//! Benchmarks for template engine
//!
//! Run with: cargo bench --bench template
//!
//! Benchmarks cover:
//! - Lexing (tokenization)
//! - Parsing (AST generation)
//! - Full render (parse + evaluate)
//! - Comparison with minijinja

use divan::{Bencher, black_box};
use facet_value::DestructuredRef;
use gingembre::lexer::Lexer;
use gingembre::parser::Parser;
use gingembre::{Context, Engine, InMemoryLoader, VArray, VObject, VString, Value};
use std::sync::Arc;

fn main() {
    divan::main();
}

/// Helper to run async code in benchmarks.
/// This is safe for benchmarks because:
/// 1. Each benchmark runs in isolation with a fresh runtime
/// 2. No shared state or complex async interactions
/// 3. We're measuring render performance, not async scheduling
fn bench_async<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
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
    ctx.set("name", Value::from("Alice"));
    ctx.set("site_name", Value::from("My Site"));
    ctx.set("created_date", Value::from("2024-01-15"));
    ctx.set("message_count", Value::from(42i64));
    ctx
}

fn loop_context() -> Context {
    let mut ctx = Context::new();
    let items: Vec<Value> = (0..10)
        .map(|i| {
            let mut item = VObject::new();
            item.insert(
                VString::from("name"),
                Value::from(format!("Item {}", i).as_str()),
            );
            item.insert(VString::from("price"), Value::from(i as f64 * 9.99));
            Value::from(item)
        })
        .collect();
    ctx.set("items", Value::from(VArray::from_iter(items)));
    ctx
}

fn complex_context() -> Context {
    let mut ctx = Context::new();

    // Page data
    let mut page = VObject::new();
    page.insert(VString::from("title"), Value::from("My Blog Post"));
    page.insert(VString::from("date"), Value::from("2024-03-15"));
    page.insert(VString::from("author"), Value::from("John Doe"));
    page.insert(
        VString::from("content"),
        Value::from("<p>This is the post content with <strong>HTML</strong>.</p>"),
    );
    page.insert(
        VString::from("tags"),
        Value::from(VArray::from_iter(vec![
            Value::from("rust"),
            Value::from("programming"),
            Value::from("web"),
        ])),
    );
    ctx.set("page", Value::from(page));

    // Site data
    let mut site = VObject::new();
    site.insert(VString::from("name"), Value::from("My Blog"));
    site.insert(VString::from("year"), Value::from(2024i64));
    ctx.set("site", Value::from(site));

    // Navigation
    let nav_links: Vec<Value> = vec![
        {
            let mut link = VObject::new();
            link.insert(VString::from("url"), Value::from("/"));
            link.insert(VString::from("label"), Value::from("Home"));
            link.insert(VString::from("active"), Value::from(false));
            Value::from(link)
        },
        {
            let mut link = VObject::new();
            link.insert(VString::from("url"), Value::from("/blog"));
            link.insert(VString::from("label"), Value::from("Blog"));
            link.insert(VString::from("active"), Value::from(true));
            Value::from(link)
        },
        {
            let mut link = VObject::new();
            link.insert(VString::from("url"), Value::from("/about"));
            link.insert(VString::from("label"), Value::from("About"));
            link.insert(VString::from("active"), Value::from(false));
            Value::from(link)
        },
    ];
    ctx.set("nav_links", Value::from(VArray::from_iter(nav_links)));

    // Related posts
    let related_posts: Vec<Value> = (0..3)
        .map(|i| {
            let mut post = VObject::new();
            post.insert(
                VString::from("url"),
                Value::from(format!("/posts/related-{}", i).as_str()),
            );
            post.insert(
                VString::from("title"),
                Value::from(format!("Related Post {}", i + 1).as_str()),
            );
            Value::from(post)
        })
        .collect();
    ctx.set(
        "related_posts",
        Value::from(VArray::from_iter(related_posts)),
    );

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
        black_box(bench_async(engine.render("bench", &ctx)))
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
        black_box(bench_async(engine.render("bench", &ctx)))
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
        black_box(bench_async(engine.render("bench", &ctx)))
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
        black_box(bench_async(engine.render("bench", &ctx)))
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
            let n = args
                .first()
                .and_then(|v| {
                    if let DestructuredRef::Number(num) = v.destructure_ref() {
                        num.to_i64()
                    } else {
                        None
                    }
                })
                .unwrap_or(0) as usize;
            Ok(Value::from(VArray::from_iter(
                (0..n).map(|i| Value::from(i as i64)),
            )))
        }),
    );

    bencher.bench(|| {
        let mut loader = InMemoryLoader::new();
        loader.add("bench", &source);
        let mut engine = Engine::new(loader);
        black_box(bench_async(engine.render("bench", &ctx)))
    });
}

// ============================================================================
// minijinja comparison benchmarks
// ============================================================================

mod minijinja_comparison {
    use super::*;
    use minijinja::{Environment, Value as MiniValue, context};

    // Helper to create minijinja context equivalent to simple_context()
    fn mj_simple_context() -> MiniValue {
        context! {
            name => "Alice",
            site_name => "My Site",
            created_date => "2024-01-15",
            message_count => 42,
        }
    }

    // Helper to create minijinja context equivalent to loop_context()
    fn mj_loop_context() -> MiniValue {
        let items: Vec<MiniValue> = (0..10)
            .map(|i| {
                context! {
                    name => format!("Item {}", i),
                    price => i as f64 * 9.99,
                }
            })
            .collect();
        context! {
            items => items,
        }
    }

    // Helper to create minijinja context equivalent to complex_context()
    fn mj_complex_context() -> MiniValue {
        let page = context! {
            title => "My Blog Post",
            date => "2024-03-15",
            author => "John Doe",
            content => "<p>This is the post content with <strong>HTML</strong>.</p>",
            tags => vec!["rust", "programming", "web"],
        };

        let site = context! {
            name => "My Blog",
            year => 2024,
        };

        let nav_links = vec![
            context! { url => "/", label => "Home", active => false },
            context! { url => "/blog", label => "Blog", active => true },
            context! { url => "/about", label => "About", active => false },
        ];

        let related_posts: Vec<MiniValue> = (0..3)
            .map(|i| {
                context! {
                    url => format!("/posts/related-{}", i),
                    title => format!("Related Post {}", i + 1),
                }
            })
            .collect();

        context! {
            page => page,
            site => site,
            nav_links => nav_links,
            related_posts => related_posts,
        }
    }

    #[divan::bench]
    fn mj_render_simple(bencher: Bencher) {
        let source = simple_text();
        let mut env = Environment::new();
        env.add_template("bench", source).unwrap();

        bencher.bench(|| {
            let tmpl = env.get_template("bench").unwrap();
            black_box(tmpl.render(context! {}))
        });
    }

    #[divan::bench]
    fn mj_render_with_variables(bencher: Bencher) {
        let source = with_variables();
        let mut env = Environment::new();
        env.add_template("bench", source).unwrap();
        let ctx = mj_simple_context();

        bencher.bench(|| {
            let tmpl = env.get_template("bench").unwrap();
            black_box(tmpl.render(&ctx))
        });
    }

    #[divan::bench]
    fn mj_render_with_loops(bencher: Bencher) {
        let source = with_loops();
        let mut env = Environment::new();
        env.add_template("bench", source).unwrap();
        let ctx = mj_loop_context();

        bencher.bench(|| {
            let tmpl = env.get_template("bench").unwrap();
            black_box(tmpl.render(&ctx))
        });
    }

    #[divan::bench]
    fn mj_render_complex(bencher: Bencher) {
        let source = complex_template();
        let mut env = Environment::new();
        env.add_template("bench", source).unwrap();
        let ctx = mj_complex_context();

        bencher.bench(|| {
            let tmpl = env.get_template("bench").unwrap();
            black_box(tmpl.render(&ctx))
        });
    }

    #[divan::bench(args = [10, 100, 1000])]
    fn mj_render_loop_scaling(bencher: Bencher, iterations: usize) {
        let source = large_loop_template(iterations);
        let mut env = Environment::new();
        env.add_template("bench", &source).unwrap();

        bencher.bench(|| {
            let tmpl = env.get_template("bench").unwrap();
            black_box(tmpl.render(context! {}))
        });
    }
}
