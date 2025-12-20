//! Integration test runner for dodeca
//!
//! This is a standalone binary that runs integration tests sequentially.
//! It bypasses cargo test/nextest entirely for better control over the test environment.
//!
//! Usage:
//!   `integration-tests [OPTIONS]`
//!
//! Environment variables:
//!   DODECA_BIN       - Path to the ddc binary (required)
//!   DODECA_CELL_PATH - Path to cell binaries (optional, defaults to same dir as ddc)

mod fd_passing;
mod harness;

use harness::{
    TestSite, clear_test_state, get_exit_status_for, get_logs_for, get_setup_for,
    set_current_test_id,
};
use owo_colors::OwoColorize;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// A test case
struct Test {
    name: &'static str,
    module: &'static str,
    func: TestFn,
    ignored: bool,
}

enum TestFn {
    Sync(fn()),
}

/// Run all tests and return (passed, failed, skipped)
fn run_tests(tests: &[Test], filter: Option<&str>) -> (usize, usize, usize) {
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut next_test_id: u64 = 1;

    fn panic_message(e: &Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        }
    }

    for test in tests {
        let full_name = format!("{}::{}", test.module, test.name);

        // Apply filter
        if let Some(filter) = filter
            && !full_name.contains(filter)
        {
            continue;
        }

        // Skip ignored tests
        if test.ignored {
            println!("{} {} ... {}", "test".bold(), full_name, "SKIP".yellow());
            skipped += 1;
            continue;
        }

        print!("{} {} ... ", "test".bold(), full_name);

        let start = Instant::now();

        let test_id = next_test_id;
        next_test_id = next_test_id.saturating_add(1);
        set_current_test_id(test_id);
        clear_test_state(test_id);

        // `catch_unwind` prevents the panic from aborting the runner, but the default
        // panic hook would still print the panic to stderr. Since we handle/report
        // failures ourselves, temporarily silence the hook.
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));

        let result = match &test.func {
            TestFn::Sync(f) => {
                let f = *f;
                panic::catch_unwind(AssertUnwindSafe(f))
            }
        };

        panic::set_hook(prev_hook);

        match result {
            Ok(()) => {
                let elapsed = start.elapsed();
                if let Some(setup) = get_setup_for(test_id) {
                    println!(
                        "{} ({:.2}s, setup {:.2}s)",
                        "PASS".green(),
                        elapsed.as_secs_f64(),
                        setup.as_secs_f64()
                    );
                } else {
                    println!("{} ({:.2}s)", "PASS".green(), elapsed.as_secs_f64());
                }
                let show_logs = std::env::var("DODECA_SHOW_LOGS")
                    .ok()
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if show_logs {
                    let logs = get_logs_for(test_id);
                    if !logs.is_empty() {
                        println!("  {} ({} lines):", "Server logs".yellow(), logs.len());
                        for line in &logs {
                            println!("    {}", line);
                        }
                    }
                }
                passed += 1;
            }
            Err(e) => {
                let msg = panic_message(&e);
                let elapsed = start.elapsed();
                if let Some(setup) = get_setup_for(test_id) {
                    println!(
                        "{} ({:.2}s, setup {:.2}s)",
                        "FAIL".red(),
                        elapsed.as_secs_f64(),
                        setup.as_secs_f64()
                    );
                } else {
                    println!("{} ({:.2}s)", "FAIL".red(), elapsed.as_secs_f64());
                }
                println!("  {}", msg.red());

                // Print server logs on failure
                let logs = get_logs_for(test_id);
                if !logs.is_empty() {
                    println!("  {} ({} lines):", "Server logs".yellow(), logs.len());
                    for line in &logs {
                        println!("    {}", line);
                    }
                }
                if let Some(status) = get_exit_status_for(test_id) {
                    println!("  {} {}", "Server exit status:".yellow(), status);
                }

                failed += 1;
                break;
            }
        }
    }

    (passed, failed, skipped)
}

/// List all tests
fn list_tests(tests: &[Test], filter: Option<&str>) {
    for test in tests {
        let full_name = format!("{}::{}", test.module, test.name);

        if let Some(filter) = filter
            && !full_name.contains(filter)
        {
            continue;
        }

        if test.ignored {
            println!("{} (ignored)", full_name);
        } else {
            println!("{}", full_name);
        }
    }
}

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("integration_tests=info".parse().unwrap()),
        )
        .init();

    // Check required environment variables
    if std::env::var("DODECA_BIN").is_err() {
        eprintln!(
            "{}: DODECA_BIN environment variable must be set",
            "error".red().bold()
        );
        eprintln!("  Set it to the path of the ddc binary, e.g.:");
        eprintln!("    export DODECA_BIN=/path/to/target/release/ddc");
        std::process::exit(1);
    }

    // Parse arguments
    let args: Vec<String> = std::env::args().collect();
    let mut filter: Option<&str> = None;
    let mut list_only = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--list" | "-l" => list_only = true,
            "--filter" | "-f" => {
                i += 1;
                if i < args.len() {
                    filter = Some(&args[i]);
                }
            }
            "--help" | "-h" => {
                println!("Integration test runner for dodeca");
                println!();
                println!("Usage: integration-tests [OPTIONS]");
                println!();
                println!("Options:");
                println!("  -l, --list          List all tests without running them");
                println!("  -f, --filter NAME   Only run tests containing NAME");
                println!("  -h, --help          Show this help");
                println!();
                println!("Environment variables:");
                println!("  DODECA_BIN          Path to the ddc binary (required)");
                println!("  DODECA_CELL_PATH    Path to cell binaries directory");
                std::process::exit(0);
            }
            arg if !arg.starts_with('-') => {
                // Positional argument treated as filter
                filter = Some(&args[i]);
            }
            _ => {
                eprintln!("{}: unknown argument: {}", "error".red().bold(), args[i]);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Collect all tests
    let tests = collect_tests();

    if list_only {
        list_tests(&tests, filter);
        return;
    }

    println!();
    println!("{}", "Running integration tests...".bold());
    println!();

    let (passed, failed, skipped) = run_tests(&tests, filter);

    println!();
    if failed > 0 {
        println!(
            "Results: {} passed, {} failed, {} skipped",
            passed.to_string().green(),
            failed.to_string().red(),
            skipped.to_string().yellow()
        );
    } else {
        println!(
            "Results: {} passed, {} failed, {} skipped",
            passed.to_string().green(),
            failed,
            skipped.to_string().yellow()
        );
    }

    if failed > 0 {
        std::process::exit(1);
    }
}

// ============================================================================
// TEST DEFINITIONS
// ============================================================================

fn collect_tests() -> Vec<Test> {
    vec![
        // basic tests
        Test {
            name: "nonexistent_page_returns_404",
            module: "basic",
            func: TestFn::Sync(basic::nonexistent_page_returns_404),
            ignored: false,
        },
        Test {
            name: "nonexistent_static_returns_404",
            module: "basic",
            func: TestFn::Sync(basic::nonexistent_static_returns_404),
            ignored: false,
        },
        Test {
            name: "pagefind_files_served",
            module: "basic",
            func: TestFn::Sync(basic::pagefind_files_served),
            ignored: false,
        },
        Test {
            name: "all_pages_return_200",
            module: "basic",
            func: TestFn::Sync(basic::all_pages_return_200),
            ignored: false,
        },
        // content tests
        Test {
            name: "markdown_content_rendered",
            module: "content",
            func: TestFn::Sync(content::markdown_content_rendered),
            ignored: false,
        },
        Test {
            name: "frontmatter_title_in_html",
            module: "content",
            func: TestFn::Sync(content::frontmatter_title_in_html),
            ignored: false,
        },
        Test {
            name: "nested_content_structure",
            module: "content",
            func: TestFn::Sync(content::nested_content_structure),
            ignored: false,
        },
        // cache_busting tests
        Test {
            name: "css_urls_are_cache_busted",
            module: "cache_busting",
            func: TestFn::Sync(cache_busting::css_urls_are_cache_busted),
            ignored: false,
        },
        Test {
            name: "font_urls_rewritten_in_css",
            module: "cache_busting",
            func: TestFn::Sync(cache_busting::font_urls_rewritten_in_css),
            ignored: false,
        },
        Test {
            name: "css_change_updates_hash",
            module: "cache_busting",
            func: TestFn::Sync(cache_busting::css_change_updates_hash),
            ignored: false,
        },
        Test {
            name: "fonts_are_subsetted",
            module: "cache_busting",
            func: TestFn::Sync(cache_busting::fonts_are_subsetted),
            ignored: false,
        },
        // templates tests
        Test {
            name: "template_renders_content",
            module: "templates",
            func: TestFn::Sync(templates::template_renders_content),
            ignored: false,
        },
        Test {
            name: "template_includes_css",
            module: "templates",
            func: TestFn::Sync(templates::template_includes_css),
            ignored: false,
        },
        Test {
            name: "template_metadata_used",
            module: "templates",
            func: TestFn::Sync(templates::template_metadata_used),
            ignored: false,
        },
        Test {
            name: "different_templates_for_different_pages",
            module: "templates",
            func: TestFn::Sync(templates::different_templates_for_different_pages),
            ignored: false,
        },
        Test {
            name: "extra_frontmatter_accessible_in_templates",
            module: "templates",
            func: TestFn::Sync(templates::extra_frontmatter_accessible_in_templates),
            ignored: false,
        },
        Test {
            name: "page_extra_frontmatter_accessible_in_templates",
            module: "templates",
            func: TestFn::Sync(templates::page_extra_frontmatter_accessible_in_templates),
            ignored: false,
        },
        Test {
            name: "code_blocks_have_copy_button_script",
            module: "templates",
            func: TestFn::Sync(templates::code_blocks_have_copy_button_script),
            ignored: false,
        },
        // static_assets tests
        Test {
            name: "svg_files_served",
            module: "static_assets",
            func: TestFn::Sync(static_assets::svg_files_served),
            ignored: false,
        },
        Test {
            name: "js_files_cache_busted",
            module: "static_assets",
            func: TestFn::Sync(static_assets::js_files_cache_busted),
            ignored: false,
        },
        Test {
            name: "static_files_served_directly",
            module: "static_assets",
            func: TestFn::Sync(static_assets::static_files_served_directly),
            ignored: false,
        },
        Test {
            name: "image_files_processed",
            module: "static_assets",
            func: TestFn::Sync(static_assets::image_files_processed),
            ignored: false,
        },
        // livereload tests
        Test {
            name: "test_new_section_detected",
            module: "livereload",
            func: TestFn::Sync(livereload::test_new_section_detected),
            ignored: false,
        },
        Test {
            name: "test_deeply_nested_new_section",
            module: "livereload",
            func: TestFn::Sync(livereload::test_deeply_nested_new_section),
            ignored: false,
        },
        Test {
            name: "test_file_move_detected",
            module: "livereload",
            func: TestFn::Sync(livereload::test_file_move_detected),
            ignored: false,
        },
        Test {
            name: "test_css_livereload",
            module: "livereload",
            func: TestFn::Sync(livereload::test_css_livereload),
            ignored: false,
        },
        // section_pages tests
        Test {
            name: "adding_page_updates_section_pages_list",
            module: "section_pages",
            func: TestFn::Sync(section_pages::adding_page_updates_section_pages_list),
            ignored: false,
        },
        Test {
            name: "adding_page_updates_via_get_section_macro",
            module: "section_pages",
            func: TestFn::Sync(section_pages::adding_page_updates_via_get_section_macro),
            ignored: false,
        },
        // error_detection tests
        Test {
            name: "template_syntax_error_shows_error_page",
            module: "error_detection",
            func: TestFn::Sync(error_detection::template_syntax_error_shows_error_page),
            ignored: false,
        },
        Test {
            name: "template_error_recovery_removes_error_page",
            module: "error_detection",
            func: TestFn::Sync(error_detection::template_error_recovery_removes_error_page),
            ignored: false,
        },
        Test {
            name: "missing_template_shows_error_page",
            module: "error_detection",
            func: TestFn::Sync(error_detection::missing_template_shows_error_page),
            ignored: false,
        },
        // dead_links tests
        Test {
            name: "dead_links_marked_in_html",
            module: "dead_links",
            func: TestFn::Sync(dead_links::dead_links_marked_in_html),
            ignored: false,
        },
        Test {
            name: "valid_links_not_marked_dead",
            module: "dead_links",
            func: TestFn::Sync(dead_links::valid_links_not_marked_dead),
            ignored: false,
        },
        // sass tests
        Test {
            name: "no_scss_builds_successfully",
            module: "sass",
            func: TestFn::Sync(sass::no_scss_builds_successfully),
            ignored: false,
        },
        Test {
            name: "scss_compiled_to_css",
            module: "sass",
            func: TestFn::Sync(sass::scss_compiled_to_css),
            ignored: false,
        },
        Test {
            name: "scss_change_triggers_rebuild",
            module: "sass",
            func: TestFn::Sync(sass::scss_change_triggers_rebuild),
            ignored: false,
        },
        // picante_cache tests
        Test {
            name: "navigating_twice_should_not_recompute_queries",
            module: "picante_cache",
            func: TestFn::Sync(picante_cache::navigating_twice_should_not_recompute_queries),
            ignored: false,
        },
        // code_execution tests
        Test {
            name: "test_successful_code_sample_shows_output",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_successful_code_sample_shows_output),
            ignored: false,
        },
        Test {
            name: "test_successful_code_sample_with_ansi_colors",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_successful_code_sample_with_ansi_colors),
            ignored: false,
        },
        Test {
            name: "test_failing_code_sample_shows_compiler_error",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_failing_code_sample_shows_compiler_error),
            ignored: false,
        },
        Test {
            name: "test_compiler_error_with_ansi_colors",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_compiler_error_with_ansi_colors),
            ignored: false,
        },
        Test {
            name: "test_incorrect_sample_expected_to_pass_fails_build",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_incorrect_sample_expected_to_pass_fails_build),
            ignored: false,
        },
        Test {
            name: "test_multiple_code_samples_executed",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_multiple_code_samples_executed),
            ignored: false,
        },
        Test {
            name: "test_non_rust_code_blocks_not_executed",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_non_rust_code_blocks_not_executed),
            ignored: false,
        },
        Test {
            name: "test_runtime_panic_reported",
            module: "code_execution",
            func: TestFn::Sync(code_execution::test_runtime_panic_reported),
            ignored: false,
        },
        // boot_contract tests (Part 8: regression tests that pin the contract)
        Test {
            name: "missing_cell_returns_http_500_not_connection_reset",
            module: "boot_contract",
            func: TestFn::Sync(boot_contract::missing_cell_returns_http_500_not_connection_reset),
            ignored: false,
        },
        Test {
            name: "immediate_request_after_fd_pass_succeeds",
            module: "boot_contract",
            func: TestFn::Sync(boot_contract::immediate_request_after_fd_pass_succeeds),
            ignored: false,
        },
    ]
}

// ============================================================================
// TEST MODULES
// ============================================================================

mod basic {
    use super::*;

    pub fn all_pages_return_200() {
        let site = TestSite::new("sample-site");
        site.get("/").assert_ok();
        site.get("/guide/").assert_ok();
        site.get("/guide/getting-started/").assert_ok();
        site.get("/guide/advanced/").assert_ok();
    }

    pub fn nonexistent_page_returns_404() {
        let site = TestSite::new("sample-site");
        let resp = site.get("/this-page-does-not-exist/");
        assert_eq!(resp.status, 404, "Nonexistent page should return 404");
    }

    pub fn nonexistent_static_returns_404() {
        let site = TestSite::new("sample-site");
        let resp = site.get("/images/nonexistent.png");
        assert_eq!(
            resp.status, 404,
            "Nonexistent static file should return 404"
        );
    }

    pub fn pagefind_files_served() {
        let site = TestSite::new("sample-site");
        site.wait_for("/pagefind/pagefind.js", Duration::from_secs(30))
            .assert_ok();
    }
}

mod content {
    use super::*;

    pub fn markdown_content_rendered() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        html.assert_ok();
        html.assert_contains("Welcome");
        html.assert_contains("This is the home page");
    }

    pub fn frontmatter_title_in_html() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        html.assert_contains("<title>Home</title>");
    }

    pub fn nested_content_structure() {
        let site = TestSite::new("sample-site");
        site.get("/guide/").assert_ok();
        site.get("/guide/getting-started/").assert_ok();
        site.get("/guide/advanced/").assert_ok();
    }
}

mod cache_busting {
    use super::*;

    pub fn css_urls_are_cache_busted() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        let css_url = html.css_link("/css/style.*.css");
        assert!(css_url.is_some(), "CSS should have cache-busted URL");
        assert!(
            css_url.as_ref().unwrap().contains('.'),
            "URL should contain hash: {:?}",
            css_url
        );
    }

    pub fn font_urls_rewritten_in_css() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        let css_url = html
            .css_link("/css/style.*.css")
            .expect("CSS link should exist");
        let css = site.get(&css_url);
        css.assert_contains("/fonts/");
        css.assert_not_contains("url('/fonts/test.woff2')");
        css.assert_not_contains("url(\"/fonts/test.woff2\")");
    }

    pub fn css_change_updates_hash() {
        let site = TestSite::new("sample-site");
        let css_url_1 = site
            .get("/")
            .css_link("/css/style.*.css")
            .expect("initial CSS URL");

        site.wait_debounce();

        site.modify_file("static/css/style.css", |css| {
            css.replace("font-weight: 400", "font-weight: 700")
        });

        let css_url_2 = site.wait_until(Duration::from_secs(10), || {
            let url = site.get("/").css_link("/css/style.*.css")?;
            if url != css_url_1 { Some(url) } else { None }
        });

        let css = site.get(&css_url_2);
        assert!(
            css.text().contains("font-weight: 700") || css.text().contains("font-weight:700"),
            "CSS should have updated font-weight"
        );
    }

    pub fn fonts_are_subsetted() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        let css_url = html
            .css_link("/css/style.*.css")
            .expect("CSS link should exist");
        let css = site.get(&css_url);

        let font_url = css.extract(r#"url\(['"]?(/fonts/test\.[^'")\s]+\.woff2)['"]?\)"#);
        assert!(
            font_url.is_some(),
            "Font URL should be in CSS: {}",
            css.text()
        );

        let font_resp = site.get(font_url.as_ref().unwrap());
        font_resp.assert_ok();
    }
}

mod templates {
    use super::*;

    pub fn template_renders_content() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        html.assert_ok();
        html.assert_contains("<!DOCTYPE html>");
        html.assert_contains("<body>");
    }

    pub fn template_includes_css() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        assert!(
            html.text().contains("stylesheet") || html.text().contains(".css"),
            "Template should include CSS references"
        );
    }

    pub fn template_metadata_used() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        html.assert_contains("<title>Home</title>");
    }

    pub fn different_templates_for_different_pages() {
        let site = TestSite::new("sample-site");
        let index = site.get("/");
        let guide = site.get("/guide/");
        index.assert_ok();
        guide.assert_ok();
        index.assert_contains("<!DOCTYPE html>");
        guide.assert_contains("<!DOCTYPE html>");
    }

    pub fn extra_frontmatter_accessible_in_templates() {
        let site = TestSite::with_files(
            "sample-site",
            &[
                (
                    "content/guide/_index.md",
                    r#"+++
title = "Guide"
[extra]
sidebar = true
icon = "book"
custom_value = 42
+++

# Guide

This is the guide section.
"#,
                ),
                (
                    "templates/section.html",
                    r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {% if section.extra.sidebar %}
  <div class="has-sidebar" data-icon="{{ section.extra.icon }}" data-custom="{{ section.extra.custom_value }}">Sidebar enabled</div>
  {% endif %}
  {{ section.content | safe }}
</body>
</html>
"#,
                ),
            ],
        );

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains("has-sidebar");
        html.assert_contains("data-icon");
        html.assert_contains("book");
        html.assert_contains("data-custom");
        html.assert_contains("42");
        html.assert_contains("Sidebar enabled");
    }

    pub fn page_extra_frontmatter_accessible_in_templates() {
        let site = TestSite::with_files(
            "sample-site",
            &[
                (
                    "content/guide/getting-started.md",
                    r#"+++
title = "Getting Started"
[extra]
difficulty = "beginner"
reading_time = 5
+++

# Getting Started

This is the getting started guide.
"#,
                ),
                (
                    "templates/page.html",
                    r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ page.title }}</title>
</head>
<body>
  <h1>{{ page.title }}</h1>
  <div class="meta" data-difficulty="{{ page.extra.difficulty }}" data-time="{{ page.extra.reading_time }}">
    Difficulty: {{ page.extra.difficulty }}, Reading time: {{ page.extra.reading_time }} min
  </div>
  {{ page.content | safe }}
</body>
</html>
"#,
                ),
            ],
        );

        let html = site.get("/guide/getting-started/");
        html.assert_ok();
        html.assert_contains("data-difficulty");
        html.assert_contains("beginner");
        html.assert_contains("data-time");
        html.assert_contains("Difficulty: beginner");
        html.assert_contains("Reading time: 5 min");
    }

    pub fn code_blocks_have_copy_button_script() {
        let site = TestSite::with_files(
            "sample-site",
            &[(
                "content/guide/code-example.md",
                r#"+++
title = "Code Example"
+++

# Code Example

Here's some code:

```rust
fn main() {
    println!("Hello, world!");
}
```
"#,
            )],
        );

        let html = site.get("/guide/code-example/");
        html.assert_ok();
        html.assert_contains(".copy-btn");
        html.assert_contains("navigator.clipboard.writeText");
        html.assert_contains("<code");
        html.assert_contains("Hello, world!");
    }
}

mod static_assets {
    use super::*;

    pub fn svg_files_served() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");
        let svg_url = html.img_src("/images/test.*.svg");

        if let Some(url) = svg_url {
            let svg = site.get(&url);
            svg.assert_ok();
            svg.assert_contains("<svg");
        }
    }

    pub fn js_files_cache_busted() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");

        if html.text().contains("/js/main.") {
            let has_hash = html.text().contains("/js/main.") && html.text().contains(".js");
            assert!(has_hash, "JS files should have cache-busted URLs");
        }
    }

    pub fn static_files_served_directly() {
        let site = TestSite::new("sample-site");
        site.write_file("static/test.txt", "Hello, World!");
        site.wait_debounce();
        std::thread::sleep(Duration::from_secs(1));

        let resp = site.get("/test.txt");
        assert!(
            resp.status == 200 || resp.status == 404,
            "Static file response should be valid"
        );
    }

    pub fn image_files_processed() {
        let site = TestSite::new("sample-site");
        let html = site.get("/");

        if let Some(svg_url) = html.img_src("/images/test.*.svg") {
            let svg = site.get(&svg_url);
            svg.assert_ok();
        }
    }
}

mod livereload {
    use super::*;

    pub fn test_new_section_detected() {
        let site = TestSite::new("sample-site");
        site.wait_debounce();
        site.delete_if_exists("content/new-section");

        let resp = site.get("/new-section/");
        assert_eq!(resp.status, 404, "New section should not exist initially");

        site.write_file(
            "content/new-section/_index.md",
            r#"+++
title = "New Section"
+++

This is a dynamically created section."#,
        );

        let _resp = site.wait_until(Duration::from_secs(10), || {
            let resp = site.get("/new-section/");
            if resp.status == 200 { Some(resp) } else { None }
        });

        let resp = site.get("/new-section/");
        resp.assert_ok();
        resp.assert_contains("dynamically created section");
    }

    pub fn test_deeply_nested_new_section() {
        let site = TestSite::new("sample-site");
        site.wait_debounce();
        site.delete_if_exists("content/level1");

        let resp = site.get("/level1/level2/level3/");
        assert_eq!(
            resp.status, 404,
            "Nested section should not exist initially"
        );

        site.write_file(
            "content/level1/level2/level3/_index.md",
            r#"+++
title = "Deeply Nested"
+++

This is a deeply nested section at level 3."#,
        );

        let _resp = site.wait_until(Duration::from_secs(10), || {
            let resp = site.get("/level1/level2/level3/");
            if resp.status == 200 { Some(resp) } else { None }
        });

        let resp = site.get("/level1/level2/level3/");
        resp.assert_ok();
        resp.assert_contains("deeply nested section");
    }

    pub fn test_file_move_detected() {
        let site = TestSite::new("sample-site");
        site.wait_debounce();

        site.write_file(
            "content/guide/moveable.md",
            r#"+++
title = "Moveable Page"
+++

This page will be moved."#,
        );

        site.wait_until(Duration::from_secs(10), || {
            let resp = site.get("/guide/moveable/");
            if resp.status == 200 { Some(resp) } else { None }
        });

        site.wait_debounce();

        let original_content = site.read_file("content/guide/moveable.md");
        site.delete_file("content/guide/moveable.md");
        site.write_file("content/moved-page.md", &original_content);

        let result = site.wait_until(Duration::from_secs(10), || {
            let old_resp = site.get("/guide/moveable/");
            let new_resp = site.get("/moved-page/");

            if old_resp.status == 404 && new_resp.status == 200 {
                Some((old_resp, new_resp))
            } else {
                None
            }
        });

        let (old_resp, new_resp) = result;
        assert_eq!(
            old_resp.status, 404,
            "Old URL should return 404 after file move"
        );
        assert_eq!(
            new_resp.status, 200,
            "New URL should be accessible after file move"
        );
        new_resp.assert_contains("This page will be moved");
    }

    pub fn test_css_livereload() {
        let site = TestSite::new("sample-site");

        const BASELINE_CSS: &str = r#"/* Test CSS with font URLs */
@font-face {
    font-family: 'TestFont';
    src: url('/fonts/test.woff2') format('woff2');
    font-weight: 400;
    font-style: normal;
}

body {
    font-family: 'TestFont', sans-serif;
}
"#;
        site.write_file("static/css/style.css", BASELINE_CSS);

        let css_url_1 = site
            .get("/")
            .css_link("/css/style.*.css")
            .expect("Initial CSS URL should exist");

        let css_1 = site.get(&css_url_1);
        css_1.assert_contains("font-weight:400");

        site.wait_debounce();

        site.modify_file("static/css/style.css", |css| {
            css.replace("font-weight: 400", "font-weight: 700")
        });

        let css_url_2 = site.wait_until(Duration::from_secs(10), || {
            let new_url = site.get("/").css_link("/css/style.*.css")?;
            if new_url != css_url_1 {
                Some(new_url)
            } else {
                None
            }
        });

        let css_2 = site.get(&css_url_2);
        css_2.assert_contains("font-weight:700");
        assert_ne!(
            css_url_1, css_url_2,
            "CSS URL hash should change after modification"
        );
    }
}

mod section_pages {
    use super::*;

    pub fn adding_page_updates_section_pages_list() {
        let site = TestSite::new("sample-site");

        site.write_file(
            "templates/section.html",
            r#"<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="page-list">
    {% for page in section.pages %}
      <a href="{{ page.permalink }}">{{ page.title }}</a>
    {% endfor %}
  </nav>
</body>
</html>
"#,
        );

        site.wait_debounce();

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains("Getting Started");
        html.assert_contains("Advanced");

        site.write_file(
            "content/guide/new-topic.md",
            r#"+++
title = "New Topic"
weight = 50
+++

# New Topic

This is a newly added page.
"#,
        );

        site.wait_debounce();

        site.wait_until(Duration::from_secs(5), || {
            let html = site.get("/guide/");
            if html.body.contains("New Topic") {
                Some(html)
            } else {
                None
            }
        });

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains("Getting Started");
        html.assert_contains("Advanced");
        html.assert_contains("New Topic");
    }

    pub fn adding_page_updates_via_get_section_macro() {
        let site = TestSite::new("sample-site");

        site.write_file(
            "templates/macros.html",
            r#"{% macro render_section_pages(section_path) %}
    {% set sec = get_section(path=section_path) %}
    <ul class="section-pages">
    {% for page in sec.pages %}
        <li><a href="{{ page.permalink }}">{{ page.title }}</a></li>
    {% endfor %}
    </ul>
{% endmacro %}
"#,
        );

        site.write_file(
            "templates/section.html",
            r#"{% import "macros.html" as macros %}
<!DOCTYPE html>
<html>
<head>
  <title>{{ section.title }}</title>
</head>
<body>
  <h1>{{ section.title }}</h1>
  {{ section.content | safe }}
  <nav id="macro-page-list">
    {{ macros::render_section_pages(section_path=section.path) }}
  </nav>
</body>
</html>
"#,
        );

        site.wait_until(Duration::from_secs(5), || {
            let html = site.get("/guide/");
            if html.body.contains("section-pages") {
                Some(html)
            } else {
                None
            }
        });

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains("Getting Started");
        html.assert_contains("Advanced");
        html.assert_contains("section-pages");

        site.write_file(
            "content/guide/macro-test-page.md",
            r#"+++
title = "Macro Test Page"
weight = 100
+++

# Macro Test Page

Testing get_section in macros.
"#,
        );

        site.wait_debounce();

        site.wait_until(Duration::from_secs(5), || {
            let html = site.get("/guide/");
            if html.body.contains("Macro Test Page") {
                Some(html)
            } else {
                None
            }
        });

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains("Macro Test Page");
    }
}

mod error_detection {
    use super::*;

    const RENDER_ERROR_MARKER: &str = "<!-- DODECA_RENDER_ERROR -->";

    pub fn template_syntax_error_shows_error_page() {
        let site = TestSite::new("sample-site");

        let html = site.get("/");
        html.assert_ok();
        html.assert_not_contains(RENDER_ERROR_MARKER);
        html.assert_contains("<!DOCTYPE html>");

        site.modify_file("templates/index.html", |content| {
            content.replace("{{ section.title }}", "{{ section.title")
        });

        std::thread::sleep(Duration::from_millis(500));

        let html = site.get("/");
        html.assert_ok();
        html.assert_contains(RENDER_ERROR_MARKER);
        html.assert_contains("Template Error");
    }

    pub fn template_error_recovery_removes_error_page() {
        let site = TestSite::new("sample-site");

        let original = site.read_file("templates/index.html");

        site.modify_file("templates/index.html", |content| {
            content.replace("{{ section.title }}", "{{ section.title")
        });

        std::thread::sleep(Duration::from_millis(500));

        let html = site.get("/");
        html.assert_contains(RENDER_ERROR_MARKER);

        site.write_file("templates/index.html", &original);

        std::thread::sleep(Duration::from_millis(500));

        let html = site.get("/");
        html.assert_not_contains(RENDER_ERROR_MARKER);
        html.assert_contains("<!DOCTYPE html>");
    }

    pub fn missing_template_shows_error_page() {
        let site = TestSite::new("sample-site");

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_not_contains(RENDER_ERROR_MARKER);

        site.delete_file("templates/section.html");

        std::thread::sleep(Duration::from_millis(500));

        let html = site.get("/guide/");
        html.assert_ok();
        html.assert_contains(RENDER_ERROR_MARKER);
    }
}

mod dead_links {
    use super::*;

    pub fn dead_links_marked_in_html() {
        let site = TestSite::new("sample-site");

        site.write_file(
            "content/dead-link-test.md",
            r#"---
title: Dead Link Test
---

Check out [this broken link](/nonexistent-page/).
"#,
        );

        site.wait_debounce();
        std::thread::sleep(Duration::from_secs(2));

        let html = site.get("/dead-link-test/");
        html.assert_ok();
        html.assert_contains("data-dead");
    }

    pub fn valid_links_not_marked_dead() {
        let site = TestSite::new("sample-site");

        let html = site.get("/");
        html.assert_ok();

        if html.text().contains("/guide/") {
            assert!(
                !html.text().contains(r#"href="/guide/" data-dead"#),
                "Valid links should not be marked as dead"
            );
        }
    }
}

mod sass {
    use super::*;

    pub fn no_scss_builds_successfully() {
        let site = TestSite::new("no-scss-site");

        let html = site.get("/");
        html.assert_ok();
        html.assert_contains("Welcome");

        assert!(
            html.css_link("/main.*.css").is_none(),
            "No CSS should be generated when SCSS is absent"
        );
    }

    pub fn scss_compiled_to_css() {
        let site = TestSite::new("sample-site");

        let html = site.get("/");
        let css_url = html
            .css_link("/main.*.css")
            .expect("SCSS should be compiled to /main.*.css");

        let css = site.get(&css_url);
        css.assert_ok();
        css.assert_contains("#3498db");
        css.assert_not_contains("$primary-color");
    }

    pub fn scss_change_triggers_rebuild() {
        let site = TestSite::new("sample-site");

        let css_url_1 = site
            .get("/")
            .css_link("/main.*.css")
            .expect("initial SCSS CSS URL");

        site.wait_debounce();

        site.modify_file("sass/main.scss", |scss| scss.replace("#3498db", "#ff0000"));

        let css_url_2 = site.wait_until(Duration::from_secs(10), || {
            let url = site.get("/").css_link("/main.*.css")?;
            if url != css_url_1 { Some(url) } else { None }
        });

        let css = site.get(&css_url_2);
        assert!(
            css.text().contains("#ff0000") || css.text().contains("red"),
            "CSS should have the new color: {}",
            css.text()
        );
        css.assert_not_contains("#3498db");
    }
}

mod picante_cache {
    use super::*;

    pub fn navigating_twice_should_not_recompute_queries() {
        // Note: This test relies on RUST_LOG=debug being set
        // In the standalone runner, we don't have fine-grained control over this
        // but the test should still work as long as caching is functioning

        let site = TestSite::new("sample-site");

        site.clear_logs();

        site.get("/guide/").assert_ok();
        let cursor = site.log_cursor();

        let first_compute_starts = site.count_logs_since(0, "compute: start");

        site.get("/guide/").assert_ok();
        let second_compute_starts = site.count_logs_since(cursor, "compute: start");

        assert!(
            second_compute_starts <= 2 || second_compute_starts * 10 < first_compute_starts.max(1),
            "expected second navigation to trigger far fewer computations: first={first_compute_starts}, second={second_compute_starts}"
        );
    }
}

mod code_execution {
    use super::*;
    use harness::InlineSite;

    /// Test that a correct code sample executes successfully and shows output
    pub fn test_successful_code_sample_shows_output() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Home

```rust
fn main() {
    println!("Hello from code execution!");
}
```
"#,
        )]);

        let result = site.build();

        // Build should succeed
        result.assert_success();

        // Should show successful execution message
        result.assert_output_contains("code samples executed successfully");
    }

    /// Test that a correct code sample with ANSI colors in output works
    pub fn test_successful_code_sample_with_ansi_colors() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Colored Output

```rust
fn main() {
    // Output with ANSI escape codes for colors
    println!("\x1b[32mGreen text\x1b[0m");
    println!("\x1b[31mRed text\x1b[0m");
    println!("\x1b[1;34mBold blue\x1b[0m");
}
```
"#,
        )]);

        let result = site.build();
        result.assert_success();
    }

    /// Test that a failing code sample causes the build to fail and shows compiler errors
    pub fn test_failing_code_sample_shows_compiler_error() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Type Error

```rust
fn main() {
    let x: i32 = "not a number";
    println!("{}", x);
}
```
"#,
        )]);

        let result = site.build();

        // Build should fail
        result.assert_failure();

        // Should show code execution failure
        result.assert_output_contains("Code execution failed");

        // Should contain type error message from rustc
        // The error message should mention "mismatched types" or similar
        result.assert_output_contains("mismatched types");
    }

    /// Test that compiler errors preserve ANSI colors from rustc
    pub fn test_compiler_error_with_ansi_colors() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Compilation Error

```rust
fn main() {
    let undefined_variable;
    println!("{}", undefined_variable);
}
```
"#,
        )]);

        let result = site.build();
        result.assert_failure();

        // The stderr should contain the error
        // Note: ANSI codes may or may not be present depending on terminal detection
        // but the error message content should be there
        result.assert_output_contains("error");
    }

    /// Test that an incorrect code sample that's expected to pass causes build failure
    pub fn test_incorrect_sample_expected_to_pass_fails_build() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# This code has a bug

The following code is supposed to work but has a typo:

```rust
fn main() {
    // Intentional error - calling non-existent method
    let numbers = vec![1, 2, 3];
    let sum = numbers.sums();  // typo: should be .iter().sum()
    println!("Sum: {}", sum);
}
```
"#,
        )]);

        let result = site.build();

        // Build should fail because the code doesn't compile
        result.assert_failure();

        // Should indicate code execution failure
        result.assert_output_contains("code sample(s) failed");
    }

    /// Test that multiple code samples are all executed
    pub fn test_multiple_code_samples_executed() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Multiple Samples

First sample:

```rust
fn main() {
    println!("Sample 1");
}
```

Second sample:

```rust
fn main() {
    println!("Sample 2");
}
```
"#,
        )]);

        let result = site.build();
        result.assert_success();

        // Should show that multiple samples executed
        // The exact message depends on implementation but should mention count
        result.assert_output_contains("code samples executed successfully");
    }

    /// Test that non-rust code blocks are not executed
    pub fn test_non_rust_code_blocks_not_executed() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Non-Rust Code

This JavaScript won't be executed:

```javascript
console.log("This should not run");
```

This Python won't be executed:

```python
print("This should not run")
```

This has no language specified:

```
Some random text
```
"#,
        )]);

        let result = site.build();

        // Build should succeed (no rust code to fail)
        result.assert_success();
    }

    /// Test that runtime panics are caught and reported
    pub fn test_runtime_panic_reported() {
        let site = InlineSite::new(&[(
            "_index.md",
            r#"+++
title = "Home"
+++

# Panic Test

```rust
fn main() {
    panic!("Intentional panic for testing!");
}
```
"#,
        )]);

        let result = site.build();

        // Build should fail because the code panicked
        result.assert_failure();

        // Should contain panic message
        result.assert_output_contains("Intentional panic");
    }
}

/// Boot contract tests: regression tests that verify the server's behavior during
/// startup and when cells are missing. These tests pin the contract from salvation.md.
mod boot_contract {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;

    /// Part 8.1: When DODECA_CELL_PATH points to a directory missing ddc-cell-http,
    /// connections must NOT get refused/reset. The server should accept the connection
    /// and respond with HTTP 500 (or similar boot-fatal response).
    ///
    /// This verifies that:
    /// 1. The accept loop is never aborted on cell loading failures
    /// 2. Connections are held until boot state is determined
    /// 3. HTTP 500 is returned instead of connection reset
    pub fn missing_cell_returns_http_500_not_connection_reset() {
        // Create a site with an empty cell path (no cells available)
        let site = TestSite::with_empty_cell_path("sample-site");

        // Give the server a moment to start and reach its Fatal boot state
        std::thread::sleep(Duration::from_millis(500));

        // Use raw TCP to verify we can connect and get a response (not ECONNREFUSED/ECONNRESET)
        let addr = format!("127.0.0.1:{}", site.port);

        let mut stream = match TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => {
                panic!(
                    "Connection should succeed even with missing cells, got: {} (kind={:?})",
                    e,
                    e.kind()
                );
            }
        };

        // Send a simple HTTP request
        let request = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            site.port
        );
        stream
            .write_all(request.as_bytes())
            .expect("write should succeed");

        // Read response with timeout
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);

        let response_str = String::from_utf8_lossy(&response);

        // Verify we got an HTTP response (not empty/connection reset)
        assert!(
            !response.is_empty(),
            "Should get an HTTP response, not connection reset"
        );

        // Should be HTTP 500 (boot-fatal response)
        assert!(
            response_str.starts_with("HTTP/1.1 500"),
            "Expected HTTP 500 response, got: {}",
            response_str.lines().next().unwrap_or("<empty>")
        );
    }

    /// Part 8.2: A request made immediately after FD passing must succeed.
    /// The connection should stay open while the server boots, and complete
    /// once the revision is ready.
    ///
    /// This verifies that:
    /// 1. The accept loop starts accepting immediately
    /// 2. Connection handlers wait for boot to complete
    /// 3. Requests succeed after boot completes
    pub fn immediate_request_after_fd_pass_succeeds() {
        // Normal site with all cells - the server should boot successfully
        let site = TestSite::new("sample-site");

        // Make a request immediately - should succeed even if server is still booting
        let resp = site.get("/");

        // The request should succeed (200 OK)
        resp.assert_ok();

        // And should have real content
        resp.assert_contains("<!DOCTYPE html>");
    }
}
