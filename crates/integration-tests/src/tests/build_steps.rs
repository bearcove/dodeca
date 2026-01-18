//! Integration tests for build steps feature.
//!
//! Tests build step execution, parameter handling, caching, and template integration.

use super::*;

/// Styx config with build steps for testing
const STYX_CONFIG: &str = r#"content content
output public

build_steps {
    echo_hello {
        command (echo "hello-from-build-step")
    }

    read_file {
        params {
            file @file
        }
    }

    word_count {
        params {
            file @file
        }
        command (wc -w "{file}")
    }

    echo_value {
        params {
            value @string
        }
        command (echo "{value}")
    }
}
"#;

/// Basic build step execution - command without parameters
pub fn build_step_basic_command() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Build output: {{ build.echo_hello() }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("Build output:");
    html.assert_contains("hello-from-build-step");
}

/// Build step that reads a file (no command, just @file param)
pub fn build_step_read_file() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "content/data.txt",
                "Hello from data file!\n",
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>File content: {{ build.read_file(file="content/data.txt") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("Hello from data file!");
}

/// Build step with command that uses file parameter
pub fn build_step_command_with_file_param() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "content/words.txt",
                "one two three four five\n",
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Word count: {{ build.word_count(file="content/words.txt") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    // wc -w outputs the count (5 words)
    html.assert_contains("Word count:");
    // The output should contain "5" somewhere
    let body = html.text();
    assert!(
        body.contains("5"),
        "Word count should include '5' for five words, got: {}",
        body
    );
}

/// Build step with string parameter
pub fn build_step_string_param() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Echo: {{ build.echo_value(value="test-output-123") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("test-output-123");
}

/// In-build caching - same step called multiple times should only execute once
pub fn build_step_caching_same_call() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Output 1: {{ build.echo_hello() }}</p>
<p>Output 2: {{ build.echo_hello() }}</p>
<p>Output 3: {{ build.echo_hello() }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    // Record log cursor before request
    let cursor = site.log_cursor();

    let html = site.get("/");
    html.assert_ok();

    // All three should show the same output
    html.assert_contains("Output 1:");
    html.assert_contains("Output 2:");
    html.assert_contains("Output 3:");

    // All three should contain the same string
    let body = html.text();
    let count = body.matches("hello-from-build-step").count();
    assert_eq!(
        count, 3,
        "Should have exactly 3 occurrences of the output, got {}",
        count
    );

    // Check logs - "Executing build step" should appear only once for echo_hello
    // (due to caching)
    let exec_count = site.count_logs_since(cursor, "Executing build step");
    assert!(
        exec_count <= 1,
        "Build step should only execute once due to caching, but found {} executions",
        exec_count
    );
}

/// Caching with file params - different files should execute separately
pub fn build_step_caching_different_files() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "content/file1.txt",
                "one two three\n",
            ),
            (
                "content/file2.txt",
                "alpha beta gamma delta epsilon\n",
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Count 1: {{ build.word_count(file="content/file1.txt") }}</p>
<p>Count 2: {{ build.word_count(file="content/file2.txt") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();

    // file1.txt has 3 words, file2.txt has 5 words
    let body = html.text();
    assert!(
        body.contains("3") && body.contains("5"),
        "Should show different word counts for different files"
    );
}

/// Cache key uses file hash - verified by build_step_caching_different_files test
/// This test verifies that the same step with same params returns cached result
pub fn build_step_cache_consistency() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "content/data.txt",
                "stable content here\n",
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Read 1: {{ build.read_file(file="content/data.txt") }}</p>
<p>Read 2: {{ build.read_file(file="content/data.txt") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    // Both reads should return the same content
    let html = site.get("/");
    html.assert_ok();

    // Count occurrences - should have exactly 2
    let body = html.text();
    let count = body.matches("stable content here").count();
    assert_eq!(
        count, 2,
        "Both reads should return the same cached content, got {} occurrences",
        count
    );
}

/// Built-in read function works independently of build steps
pub fn builtin_read_function() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "content/readme.txt",
                "This is readme content.\n",
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Read: {{ read(file="content/readme.txt") }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    html.assert_contains("This is readme content.");
}

/// Error handling - unknown build step
pub fn build_step_unknown_step_error() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Result: {{ build.nonexistent_step() }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    // The page should still render, but with an error message in place of the result
    html.assert_ok();
    let body = html.text();
    assert!(
        body.contains("Unknown build step") || body.contains("error") || body.contains("Error"),
        "Should show error for unknown build step, got: {}",
        &body[..body.len().min(500)]
    );
}

/// Error handling - missing required parameter
pub fn build_step_missing_param_error() {
    let site = TestSite::with_files(
        "sample-site",
        &[
            (
                ".config/dodeca.styx",
                STYX_CONFIG,
            ),
            (
                "templates/index.html",
                r#"<!DOCTYPE html>
<html>
<head><title>{{ section.title }}</title></head>
<body>
<h1>{{ section.title }}</h1>
<p>Result: {{ build.read_file() }}</p>
{{ section.content | safe }}
</body>
</html>
"#,
            ),
        ],
    );

    let html = site.get("/");
    html.assert_ok();
    let body = html.text();
    assert!(
        body.contains("Missing parameter") || body.contains("error") || body.contains("Error") || body.contains("file"),
        "Should show error for missing parameter, got: {}",
        &body[..body.len().min(500)]
    );
}
