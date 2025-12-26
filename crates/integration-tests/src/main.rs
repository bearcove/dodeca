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
mod tests;

use harness::{
    clear_test_state, get_exit_status_for, get_logs_for, get_setup_for, set_current_test_id,
};
use owo_colors::OwoColorize;
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;
use tests::*;
use tracing_subscriber::layer::SubscriberExt;

/// A test case
struct Test {
    name: &'static str,
    module: &'static str,
    func: fn(),
    ignored: bool,
}

/// Shared test ID for tracing layer
static CURRENT_TRACING_TEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Custom tracing layer that captures logs for the current test
#[derive(Debug)]
struct TestTracingLayer;

impl TestTracingLayer {
    fn new() -> Self {
        Self
    }

    fn set_test_id(id: u64) {
        CURRENT_TRACING_TEST_ID.store(id, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<S> tracing_subscriber::Layer<S> for TestTracingLayer
where
    S: tracing::Subscriber,
{
    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        let test_id = CURRENT_TRACING_TEST_ID.load(std::sync::atomic::Ordering::Relaxed);
        if test_id > 0 {
            // During tests, capture everything at DEBUG level and above
            *metadata.level() <= tracing::Level::DEBUG
        } else {
            false
        }
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let test_id = CURRENT_TRACING_TEST_ID.load(std::sync::atomic::Ordering::Relaxed);
        if test_id == 0 {
            return; // No test currently running
        }

        // Filter out noisy ureq logs - check message content for specific patterns
        let target = event.metadata().target();
        if target == "log" {
            let mut visitor = LogVisitor::new();
            event.record(&mut visitor);
            let message = &visitor.message;

            // Filter out specific noisy ureq patterns
            if message.contains("Call<")
                || message.contains("GET http://")
                || message.contains("POST http://")
                || message.contains("PUT http://")
                || message.contains("DELETE http://")
                || message.contains("Resolved: ArrayVec")
                || message.contains("Connected TcpStream")
                || message.contains("Response { status:")
                || message.contains("Pool gone:")
                || message.starts_with("Request {")
            {
                return;
            }
        }

        // Format the event similar to how tracing_subscriber::fmt would
        let metadata = event.metadata();
        let mut visitor = LogVisitor::new();
        event.record(&mut visitor);

        let level = match *metadata.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            tracing::Level::INFO => "INFO",
            tracing::Level::DEBUG => "DEBUG",
            tracing::Level::TRACE => "TRACE",
        };

        // Push to the test logs using the structured system with fields
        harness::push_test_log_with_fields(
            test_id,
            level,
            metadata.target(),
            &visitor.message,
            visitor.fields,
        );
    }
}

struct LogVisitor {
    message: String,
    fields: std::collections::HashMap<String, String>,
}

impl LogVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: std::collections::HashMap::new(),
        }
    }
}

impl tracing::field::Visit for LogVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let formatted_value = format!("{:?}", value);
        if field.name() == "message" {
            self.message = formatted_value;
        } else {
            // Store structured fields separately
            self.fields
                .insert(field.name().to_string(), formatted_value);
        }
    }
}

/// Run all tests and return (passed, failed, skipped)
fn run_tests(tests: Vec<Test>, filter: Option<&str>) -> (usize, usize, usize) {
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

        // Clear any previous state for this test ID first
        clear_test_state(test_id);

        // Set test ID in the tracing layer FIRST
        TestTracingLayer::set_test_id(test_id);

        // Create new test state (this must come after clearing)
        set_current_test_id(test_id);

        // `catch_unwind` prevents the panic from aborting the runner, but the default
        // panic hook would still print the panic to stderr. Since we handle/report
        // failures ourselves, temporarily silence the hook.
        let prev_hook = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));

        let result = {
            let f = test.func;
            panic::catch_unwind(AssertUnwindSafe(f))
        };

        panic::set_hook(prev_hook);

        // Reset tracing layer test ID
        TestTracingLayer::set_test_id(0);

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
    // Initialize tracing with only our custom layer for test log capture
    let test_layer = TestTracingLayer::new();

    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry().with(test_layer).init();

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

    let (passed, failed, skipped) = run_tests(tests, filter);

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
            func: basic::nonexistent_page_returns_404,
            ignored: false,
        },
        Test {
            name: "nonexistent_static_returns_404",
            module: "basic",
            func: basic::nonexistent_static_returns_404,
            ignored: false,
        },
        Test {
            name: "pagefind_files_served",
            module: "basic",
            func: basic::pagefind_files_served,
            ignored: false,
        },
        Test {
            name: "all_pages_return_200",
            module: "basic",
            func: basic::all_pages_return_200,
            ignored: false,
        },
        // content tests
        Test {
            name: "markdown_content_rendered",
            module: "content",
            func: content::markdown_content_rendered,
            ignored: false,
        },
        Test {
            name: "frontmatter_title_in_html",
            module: "content",
            func: content::frontmatter_title_in_html,
            ignored: false,
        },
        Test {
            name: "nested_content_structure",
            module: "content",
            func: content::nested_content_structure,
            ignored: false,
        },
        // cache_busting tests
        Test {
            name: "css_urls_are_cache_busted",
            module: "cache_busting",
            func: cache_busting::css_urls_are_cache_busted,
            ignored: false,
        },
        Test {
            name: "font_urls_rewritten_in_css",
            module: "cache_busting",
            func: cache_busting::font_urls_rewritten_in_css,
            ignored: false,
        },
        Test {
            name: "css_change_updates_hash",
            module: "cache_busting",
            func: cache_busting::css_change_updates_hash,
            ignored: false,
        },
        Test {
            name: "fonts_are_subsetted",
            module: "cache_busting",
            func: cache_busting::fonts_are_subsetted,
            ignored: false,
        },
        // templates tests
        Test {
            name: "template_renders_content",
            module: "templates",
            func: templates::template_renders_content,
            ignored: false,
        },
        Test {
            name: "template_includes_css",
            module: "templates",
            func: templates::template_includes_css,
            ignored: false,
        },
        Test {
            name: "template_metadata_used",
            module: "templates",
            func: templates::template_metadata_used,
            ignored: false,
        },
        Test {
            name: "different_templates_for_different_pages",
            module: "templates",
            func: templates::different_templates_for_different_pages,
            ignored: false,
        },
        Test {
            name: "extra_frontmatter_accessible_in_templates",
            module: "templates",
            func: templates::extra_frontmatter_accessible_in_templates,
            ignored: false,
        },
        Test {
            name: "page_extra_frontmatter_accessible_in_templates",
            module: "templates",
            func: templates::page_extra_frontmatter_accessible_in_templates,
            ignored: false,
        },
        Test {
            name: "code_blocks_have_copy_button_script",
            module: "templates",
            func: templates::code_blocks_have_copy_button_script,
            ignored: false,
        },
        // static_assets tests
        Test {
            name: "svg_files_served",
            module: "static_assets",
            func: static_assets::svg_files_served,
            ignored: false,
        },
        Test {
            name: "js_files_cache_busted",
            module: "static_assets",
            func: static_assets::js_files_cache_busted,
            ignored: false,
        },
        Test {
            name: "static_files_served_directly",
            module: "static_assets",
            func: static_assets::static_files_served_directly,
            ignored: false,
        },
        Test {
            name: "image_files_processed",
            module: "static_assets",
            func: static_assets::image_files_processed,
            ignored: false,
        },
        // livereload tests
        Test {
            name: "test_new_section_detected",
            module: "livereload",
            func: livereload::test_new_section_detected,
            ignored: false,
        },
        Test {
            name: "test_deeply_nested_new_section",
            module: "livereload",
            func: livereload::test_deeply_nested_new_section,
            ignored: false,
        },
        Test {
            name: "test_file_move_detected",
            module: "livereload",
            func: livereload::test_file_move_detected,
            ignored: false,
        },
        Test {
            name: "test_css_livereload",
            module: "livereload",
            func: livereload::test_css_livereload,
            ignored: false,
        },
        // section_pages tests
        Test {
            name: "adding_page_updates_section_pages_list",
            module: "section_pages",
            func: section_pages::adding_page_updates_section_pages_list,
            ignored: false,
        },
        Test {
            name: "adding_page_updates_via_get_section_macro",
            module: "section_pages",
            func: section_pages::adding_page_updates_via_get_section_macro,
            ignored: false,
        },
        // error_detection tests
        Test {
            name: "template_syntax_error_shows_error_page",
            module: "error_detection",
            func: error_detection::template_syntax_error_shows_error_page,
            ignored: false,
        },
        Test {
            name: "template_error_recovery_removes_error_page",
            module: "error_detection",
            func: error_detection::template_error_recovery_removes_error_page,
            ignored: false,
        },
        Test {
            name: "missing_template_shows_error_page",
            module: "error_detection",
            func: error_detection::missing_template_shows_error_page,
            ignored: false,
        },
        // dead_links tests
        Test {
            name: "dead_links_marked_in_html",
            module: "dead_links",
            func: dead_links::dead_links_marked_in_html,
            ignored: false,
        },
        Test {
            name: "valid_links_not_marked_dead",
            module: "dead_links",
            func: dead_links::valid_links_not_marked_dead,
            ignored: false,
        },
        // sass tests
        Test {
            name: "no_scss_builds_successfully",
            module: "sass",
            func: sass::no_scss_builds_successfully,
            ignored: false,
        },
        Test {
            name: "scss_compiled_to_css",
            module: "sass",
            func: sass::scss_compiled_to_css,
            ignored: false,
        },
        Test {
            name: "scss_change_triggers_rebuild",
            module: "sass",
            func: sass::scss_change_triggers_rebuild,
            ignored: false,
        },
        // picante_cache tests
        Test {
            name: "navigating_twice_should_not_recompute_queries",
            module: "picante_cache",
            func: picante_cache::navigating_twice_should_not_recompute_queries,
            ignored: false,
        },
        // code_execution tests
        Test {
            name: "test_successful_code_sample_shows_output",
            module: "code_execution",
            func: code_execution::test_successful_code_sample_shows_output,
            ignored: false,
        },
        Test {
            name: "test_successful_code_sample_with_ansi_colors",
            module: "code_execution",
            func: code_execution::test_successful_code_sample_with_ansi_colors,
            ignored: false,
        },
        Test {
            name: "test_failing_code_sample_shows_compiler_error",
            module: "code_execution",
            func: code_execution::test_failing_code_sample_shows_compiler_error,
            ignored: false,
        },
        Test {
            name: "test_compiler_error_with_ansi_colors",
            module: "code_execution",
            func: code_execution::test_compiler_error_with_ansi_colors,
            ignored: false,
        },
        Test {
            name: "test_incorrect_sample_expected_to_pass_fails_build",
            module: "code_execution",
            func: code_execution::test_incorrect_sample_expected_to_pass_fails_build,
            ignored: false,
        },
        Test {
            name: "test_multiple_code_samples_executed",
            module: "code_execution",
            func: code_execution::test_multiple_code_samples_executed,
            ignored: false,
        },
        Test {
            name: "test_non_rust_code_blocks_not_executed",
            module: "code_execution",
            func: code_execution::test_non_rust_code_blocks_not_executed,
            ignored: false,
        },
        Test {
            name: "test_runtime_panic_reported",
            module: "code_execution",
            func: code_execution::test_runtime_panic_reported,
            ignored: false,
        },
        // boot_contract tests (Part 8: regression tests that pin the contract)
        Test {
            name: "immediate_request_after_fd_pass_succeeds",
            module: "boot_contract",
            func: boot_contract::immediate_request_after_fd_pass_succeeds,
            ignored: false,
        },
    ]
}
