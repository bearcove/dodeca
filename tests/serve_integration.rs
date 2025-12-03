//! Integration tests for the serve functionality
//!
//! These tests verify that all pages are accessible when serving a site.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use regex::Regex;

/// Server handle with the actual bound port
struct ServerHandle {
    child: Child,
    port: u16,
    /// Root of the isolated fixture used by this server
    fixture_dir: PathBuf,
    /// Drops last to ensure fixture persists until server is killed
    _fixture_guard: TempDirGuard,
}

/// Temporary directory that deletes itself on drop
struct TempDirGuard {
    path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Recursively copy a directory (files only, preserves relative layout)
fn copy_dir_recursive(src: &Path, dst: &Path) -> color_eyre::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

/// Create an isolated copy of the fixture directory with its own .cache
fn create_isolated_fixture(fixture_rel: &str) -> color_eyre::Result<(PathBuf, TempDirGuard)> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_dir.join("tests/fixtures").join(fixture_rel);

    let uniq = format!(
        "dodeca-fixture-{}-{}",
        fixture_rel.replace('/', "-"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dst = std::env::temp_dir().join(uniq);

    copy_dir_recursive(&src, &dst)?;
    // Ensure .cache exists and is empty
    let cache_dir = dst.join(".cache");
    let _ = fs::remove_dir_all(&cache_dir);
    fs::create_dir_all(&cache_dir)?;

    Ok((dst.clone(), TempDirGuard { path: dst }))
}

/// Start the server with given args, wait for LISTENING_PORT=...
fn start_server_with_args(args: &[&str], fixture_dir: PathBuf, fixture_guard: TempDirGuard) -> ServerHandle {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ddc"))
        .args(args)
        .stdout(Stdio::piped())
        // Inherit stderr so verbose tracing doesn't block the child process
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to start server");

    // Read stdout to find LISTENING_PORT=...
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let mut reader = BufReader::new(stdout);

    let port_regex = Regex::new(r"LISTENING_PORT=(\d+)").unwrap();
    let mut port = None;

    for line in reader.by_ref().lines() {
        let line = line.expect("Failed to read line");
        if let Some(caps) = port_regex.captures(&line) {
            port = Some(caps[1].parse::<u16>().expect("Invalid port number"));
            break;
        }
    }

    let port = port.expect("Server did not print LISTENING_PORT");

    // Keep draining stdout so the child never hits a broken pipe when it logs,
    // and mirror lines to stderr for debugging in test output.
    std::thread::spawn(move || {
        for line in reader.lines() {
            if let Ok(line) = line {
                eprintln!("[server] {line}");
            } else {
                break;
            }
        }
    });

    ServerHandle { child, port, fixture_dir, _fixture_guard: fixture_guard }
}

/// Start the server in plain mode (no TUI)
fn start_server(fixture_path: &str) -> ServerHandle {
    let (fixture_dir, guard) = create_isolated_fixture(fixture_path).expect("copy fixture");
    let fixture_str = fixture_dir.to_string_lossy().to_string();

    start_server_with_args(&["serve", &fixture_str, "--no-tui", "-p", "0"], fixture_dir, guard)
}

/// Wait for server to be ready by polling the root endpoint
fn wait_for_server(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let client = reqwest::blocking::Client::new();

    while start.elapsed() < timeout {
        if let Ok(resp) = client.get(format!("http://127.0.0.1:{}/", port)).send()
            && resp.status().is_success()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Test that all expected pages return 200
#[test_log::test]
fn test_all_pages_accessible() {
    // Start the server (port auto-selected)
    let server = start_server("sample-site");
    let port = server.port;

    // Wait for server to be ready
    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // Test all expected pages
    let pages = [
        ("/", "Home page"),
        ("/guide/", "Guide section"),
        ("/guide/getting-started/", "Getting started page"),
        ("/guide/advanced/", "Advanced page"),
    ];

    let mut failures = Vec::new();

    for (path, description) in pages {
        let url = format!("http://127.0.0.1:{}{}", port, path);
        match client.get(&url).send() {
            Ok(resp) => {
                if !resp.status().is_success() {
                    failures.push(format!(
                        "{} ({}) returned status {}",
                        description,
                        path,
                        resp.status()
                    ));
                }
            }
            Err(e) => {
                failures.push(format!("{} ({}) failed: {}", description, path, e));
            }
        }
    }

    // Kill the server and wait to avoid zombie process
    // Server cleanup handled by Drop

    // Report all failures at once
    if !failures.is_empty() {
        panic!("Page accessibility failures:\n{}", failures.join("\n"));
    }
}

/// Test that CSS files have their internal URLs rewritten to cache-busted versions
#[test_log::test]
fn test_css_font_urls_are_rewritten() {
    let server = start_server("sample-site");
    let port = server.port;

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // 1. Fetch the HTML and find the CSS URL
    let html = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML")
        .text()
        .expect("Failed to read HTML body");

    // Find the cache-busted CSS URL in HTML (e.g., /css/style.0a3dec47oc21.css)
    // Note: minified HTML may have unquoted attributes like href=/css/style.123.css
    // Hash uses dodeca alphabet: 0123456acdeo (base12)
    let css_url_re = Regex::new(r#"href=["']?(/css/style\.[0-6acdeo]+\.css)["']?"#).unwrap();
    let css_url = css_url_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or_else(|| panic!("Could not find cache-busted CSS URL in HTML.\nHTML:\n{html}"));

    // 2. Fetch the CSS file
    let css = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url))
        .send()
        .expect("Failed to fetch CSS")
        .text()
        .expect("Failed to read CSS body");

    // 3. Verify font URLs in CSS are cache-busted (not the original /fonts/test.woff2)
    assert!(
        !css.contains("/fonts/test.woff2"),
        "CSS still contains original font URL. CSS content:\n{css}"
    );

    // Font URL should be cache-busted (e.g., /fonts/test.0a3dec47oc21.woff2)
    // Hash uses dodeca alphabet: 0123456acdeo (base12)
    let font_url_re = Regex::new(r"/fonts/test\.[0-6acdeo]+\.woff2").unwrap();
    assert!(
        font_url_re.is_match(&css),
        "CSS does not contain cache-busted font URL. CSS content:\n{css}"
    );

    // 4. Verify the cache-busted font is accessible
    let font_url = font_url_re
        .find(&css)
        .map(|m| m.as_str())
        .expect("Could not find font URL");

    let font_resp = client
        .get(format!("http://127.0.0.1:{}{}", port, font_url))
        .send()
        .expect("Failed to fetch font");

    assert!(
        font_resp.status().is_success(),
        "Font URL {} returned status {}",
        font_url,
        font_resp.status()
    );

    // Server cleanup handled by Drop
}

/// Test that CSS changes are picked up by the file watcher (livereload)
#[test_log::test]
fn test_css_livereload() {
    let server = start_server("sample-site");
    let css_file_path = server.fixture_dir.join("static/css/style.css");
    let port = server.port;

    // Ensure baseline CSS content inside the isolated fixture
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
    fs::write(&css_file_path, BASELINE_CSS).expect("Failed to reset CSS fixture");

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // 1. Fetch the HTML and find the CSS URL
    let html = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML")
        .text()
        .expect("Failed to read HTML body");

    // Find the cache-busted CSS URL in HTML
    // Hash uses dodeca alphabet: 0123456acdeo (base12)
    let css_url_re = Regex::new(r#"href=[\"']?(/css/style\.[0-6acdeo]+\.css)[\"']?"#).unwrap();
    let css_url_1 = css_url_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .expect("Could not find CSS URL in HTML");

    // 2. Fetch the initial CSS content
    let css_1 = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url_1))
        .send()
        .expect("Failed to fetch CSS")
        .text()
        .expect("Failed to read CSS body");

    assert!(
        css_1.contains("font-weight: 400") || css_1.contains("font-weight:400"),
        "Initial CSS should contain font-weight: 400. CSS content:\n{css_1}"
    );

    // Give the watcher debounce window time to clear before the next write
    std::thread::sleep(Duration::from_millis(200));

    // 3. Modify the CSS file
    let original_css = std::fs::read_to_string(&css_file_path).expect("Failed to read CSS file");
    let modified_css = original_css.replace("font-weight: 400", "font-weight: 700");
    std::fs::write(&css_file_path, &modified_css).expect("Failed to write CSS file");

    // 4. Poll until file watcher reloads (up to 10s)
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut css_url_2 = css_url_1.clone();
    let mut css_2 = css_1.clone();

    while Instant::now() < deadline {
        let html_2 = client
            .get(format!("http://127.0.0.1:{}/", port))
            .send()
            .expect("Failed to fetch HTML after CSS change")
            .text()
            .expect("Failed to read HTML body");

        if let Some(new_url) = css_url_re
            .captures(&html_2)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
        {
            css_url_2 = new_url;
        }

        css_2 = client
            .get(format!("http://127.0.0.1:{}{}", port, css_url_2))
            .send()
            .expect("Failed to fetch CSS after change")
            .text()
            .expect("Failed to read CSS body");

        if (css_url_2 != css_url_1)
            && (css_2.contains("font-weight: 700") || css_2.contains("font-weight:700"))
        {
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // The CSS URL hash should have changed
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification.\nBefore: {css_url_1}\nAfter: {css_url_2}"
    );

    // 6. Fetch the new CSS content
    assert!(
        css_2.contains("font-weight: 700") || css_2.contains("font-weight:700"),
        "Updated CSS should contain font-weight: 700. CSS content:\n{css_2}"
    );

    // Server cleanup handled by Drop
}

/// Start the server in TUI mode
fn start_server_tui(fixture_path: &str) -> ServerHandle {
    let (fixture_dir, guard) = create_isolated_fixture(fixture_path).expect("copy fixture");
    let fixture_str = fixture_dir.to_string_lossy().to_string();
    start_server_with_args(&["serve", &fixture_str, "--force-tui", "-p", "0"], fixture_dir, guard)
}

/// Test that CSS changes are picked up in TUI mode (livereload)
/// NOTE: Requires real PTY - ignored until we add portable-pty testing
#[test_log::test]
#[ignore = "TUI mode requires real terminal, needs PTY testing setup"]
fn test_css_livereload_tui_mode() {
    let server = start_server_tui("sample-site");
    let port = server.port;
    let css_file_path = server.fixture_dir.join("static/css/style.css");

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // 1. Fetch the HTML and find the CSS URL
    let html = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML")
        .text()
        .expect("Failed to read HTML body");

    // Find the cache-busted CSS URL in HTML
    // Hash uses dodeca alphabet: 0123456acdeo (base12)
    let css_url_re = Regex::new(r#"href=[\"']?(/css/style\.[0-6acdeo]+\.css)[\"']?"#).unwrap();
    let css_url_1 = css_url_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .expect("Could not find CSS URL in HTML");

    // 2. Fetch the initial CSS content
    let css_1 = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url_1))
        .send()
        .expect("Failed to fetch CSS")
        .text()
        .expect("Failed to read CSS body");

    assert!(
        css_1.contains("font-weight: 400") || css_1.contains("font-weight:400"),
        "Initial CSS should contain font-weight: 400. CSS content:\n{css_1}"
    );

    // 3. Modify the CSS file
    let original_css = std::fs::read_to_string(&css_file_path).expect("Failed to read CSS file");
    let modified_css = original_css.replace("font-weight: 400", "font-weight: 700");
    std::fs::write(&css_file_path, &modified_css).expect("Failed to write CSS file");

    // 4. Wait for file watcher to pick up the change
    std::thread::sleep(Duration::from_millis(500));

    // 5. Fetch the HTML again to get the new CSS URL (hash should change)
    let html_2 = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML after CSS change")
        .text()
        .expect("Failed to read HTML body");

    let css_url_2 = css_url_re
        .captures(&html_2)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .expect("Could not find CSS URL in HTML after change");

    // The CSS URL hash should have changed
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification.\nBefore: {css_url_1}\nAfter: {css_url_2}"
    );

    // 6. Fetch the new CSS content
    let css_2 = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url_2))
        .send()
        .expect("Failed to fetch CSS after change")
        .text()
        .expect("Failed to read CSS body");

    assert!(
        css_2.contains("font-weight: 700") || css_2.contains("font-weight:700"),
        "Updated CSS should contain font-weight: 700. CSS content:\n{css_2}"
    );

    // 7. Restore the original CSS
    std::fs::write(&css_file_path, original_css).expect("Failed to restore CSS file");

    // Server cleanup handled by Drop
}

/// Start the server in TUI mode from within the project directory (like user does)
fn start_server_tui_in_dir(dir: &str) -> ServerHandle {
    // Copy target dir to isolated temp to avoid shared .cache
    let src = PathBuf::from(dir);
    let uniq = format!(
        "dodeca-docs-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dst = std::env::temp_dir().join(uniq);
    copy_dir_recursive(&src, &dst).expect("copy docs dir");
    let cache_dir = dst.join(".cache");
    let _ = fs::remove_dir_all(&cache_dir);
    fs::create_dir_all(&cache_dir).expect("create cache");

    let mut child = Command::new(env!("CARGO_BIN_EXE_ddc"))
        .args(["serve", "--force-tui", "-p", "0"])
        .current_dir(&dst)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start server");

    // Read stdout to find LISTENING_PORT=...
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let reader = BufReader::new(stdout);
    let port_regex = Regex::new(r"LISTENING_PORT=(\d+)").unwrap();
    let mut port = None;

    for line in reader.lines() {
        let line = line.expect("Failed to read line");
        if let Some(caps) = port_regex.captures(&line) {
            port = Some(caps[1].parse::<u16>().expect("Invalid port number"));
            break;
        }
    }

    let port = port.expect("Server did not print LISTENING_PORT");
    ServerHandle { child, port, fixture_dir: dst.clone(), _fixture_guard: TempDirGuard { path: dst } }
}

/// Test CSS livereload in TUI mode using docs directory (mimics user workflow)
/// NOTE: Requires real PTY - ignored until we add portable-pty testing
#[test_log::test]
#[ignore = "TUI mode requires real terminal, needs PTY testing setup"]
fn test_css_livereload_tui_docs() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let docs_dir = format!("{}/docs", manifest_dir);

    let server = start_server_tui_in_dir(&docs_dir);
    let port = server.port;
    let css_file_path = server.fixture_dir.join("static/css/style.css");

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // 1. Fetch the HTML and find the CSS URL
    let html = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML")
        .text()
        .expect("Failed to read HTML body");

    // Find the cache-busted CSS URL in HTML
    // Hash uses dodeca alphabet: 0123456acdeo (base12)
    let css_url_re = Regex::new(r#"href=[\"']?(/css/style\.[0-6acdeo]+\.css)[\"']?"#).unwrap();
    let css_url_1 = css_url_re
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .expect("Could not find CSS URL in HTML");

    // 2. Fetch the initial CSS content
    let css_1 = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url_1))
        .send()
        .expect("Failed to fetch CSS")
        .text()
        .expect("Failed to read CSS body");

    assert!(
        css_1.contains("font-weight: 400") || css_1.contains("font-weight:400"),
        "Initial CSS should contain font-weight: 400. CSS content:\n{css_1}"
    );

    // 3. Modify the CSS file
    let original_css = std::fs::read_to_string(&css_file_path).expect("Failed to read CSS file");
    let modified_css = original_css.replace("font-weight: 400", "font-weight: 700");
    std::fs::write(&css_file_path, &modified_css).expect("Failed to write CSS file");

    // 4. Wait for file watcher to pick up the change
    std::thread::sleep(Duration::from_millis(500));

    // 5. Fetch the HTML again to get the new CSS URL (hash should change)
    let html_2 = client
        .get(format!("http://127.0.0.1:{}/", port))
        .send()
        .expect("Failed to fetch HTML after CSS change")
        .text()
        .expect("Failed to read HTML body");

    let css_url_2 = css_url_re
        .captures(&html_2)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .expect("Could not find CSS URL in HTML after change");

    // The CSS URL hash should have changed
    assert_ne!(
        css_url_1, css_url_2,
        "CSS URL hash should change after modification.\nBefore: {css_url_1}\nAfter: {css_url_2}"
    );

    // 6. Fetch the new CSS content
    let css_2 = client
        .get(format!("http://127.0.0.1:{}{}", port, css_url_2))
        .send()
        .expect("Failed to fetch CSS after change")
        .text()
        .expect("Failed to read CSS body");

    assert!(
        css_2.contains("font-weight: 700") || css_2.contains("font-weight:700"),
        "Updated CSS should contain font-weight: 700. CSS content:\n{css_2}"
    );

    // 7. Restore the original CSS
    std::fs::write(&css_file_path, original_css).expect("Failed to restore CSS file");

    // Server cleanup handled by Drop
}

/// Test that new content files are detected and served (issue #7)
#[test_log::test]
fn test_new_content_file_detected() {
    let server = start_server("sample-site");
    let new_page_path = server.fixture_dir.join("content/new-page.md");
    let port = server.port;

    // Ensure the new page doesn't exist before starting
    let _ = std::fs::remove_file(&new_page_path);

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // Verify the new page doesn't exist yet
    let resp = client
        .get(format!("http://127.0.0.1:{}/new-page/", port))
        .send()
        .expect("Failed to fetch new page");
    assert_eq!(resp.status().as_u16(), 404, "New page should not exist initially");

    // Create the new page
    let new_page_content = r#"+++
title = "New Page"
+++

This is a dynamically created page.
"#;
    std::fs::write(&new_page_path, new_page_content).expect("Failed to create new page");

    // Wait for file watcher to pick up the change (poll up to 5s)
    let url = format!("http://127.0.0.1:{}/new-page/", port);
    // Allow generous time because file watcher latency can vary on CI
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last_status = None;
    let resp = loop {
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => break resp,
            Ok(resp) => {
                last_status = Some(resp.status());
            }
            Err(_) => {}
        }

        if Instant::now() >= deadline {
            let status = last_status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "no response".to_string());
            panic!(
                "New page should be accessible after creation within 5s, last status: {}",
                status
            );
        }

        std::thread::sleep(Duration::from_millis(100));
    };

    assert!(
        resp.status().is_success(),
        "New page should be accessible after creation, got status {}",
        resp.status()
    );

    let body = resp.text().expect("Failed to read body");
    assert!(
        body.contains("dynamically created"),
        "New page content should be present. Body:\n{body}"
    );

    // Clean up
    let _ = std::fs::remove_file(&new_page_path);

    // Server cleanup handled by Drop
}

/// Test that deleted content files result in 404 (issue #7)
#[test_log::test]
fn test_deleted_content_file_returns_404() {
    let server = start_server("sample-site");
    let temp_page_path = server.fixture_dir.join("content/temp-page.md");
    let port = server.port;

    // Create a page before starting the server
    let page_content = r#"+++
title = "Temporary Page"
+++

This page will be deleted.
"#;
    std::fs::write(&temp_page_path, page_content).expect("Failed to create temp page");

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // Verify the page exists initially
    let resp = client
        .get(format!("http://127.0.0.1:{}/temp-page/", port))
        .send()
        .expect("Failed to fetch temp page");
    assert!(
        resp.status().is_success(),
        "Temp page should exist initially, got status {}",
        resp.status()
    );

    // Delete the page
    std::fs::remove_file(&temp_page_path).expect("Failed to delete temp page");

    // Wait for file watcher to pick up the change (poll up to 10s)
    let url = format!("http://127.0.0.1:{}/temp-page/", port);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let resp = client.get(&url).send().expect("Failed to fetch temp page after deletion");
        let status = resp.status();
        if status.as_u16() == 404 {
            break;
        }

        if Instant::now() >= deadline {
            panic!(
                "Deleted page should return 404 within 10s, last status {}",
                status
            );
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    // Clean up (in case test failed before deletion)
    let _ = std::fs::remove_file(&temp_page_path);

    // Server cleanup handled by Drop
}

/// Test that new section files (_index.md) are detected
/// NOTE: This test is currently ignored because notify-rs doesn't watch newly created
/// subdirectories by default. New files in EXISTING directories work fine.
#[test_log::test]
#[ignore = "File watcher doesn't detect files in newly created subdirectories"]
fn test_new_section_detected() {
    let server = start_server("sample-site");
    let new_section_dir = server.fixture_dir.join("content/new-section");
    let new_section_path = new_section_dir.join("_index.md");
    let port = server.port;

    // Ensure the new section doesn't exist before starting
    let _ = std::fs::remove_dir_all(&new_section_dir);

    let ready = wait_for_server(port, Duration::from_secs(30));
    assert!(ready, "Server did not start within timeout");

    let client = reqwest::blocking::Client::new();

    // Verify the new section doesn't exist yet
    let resp = client
        .get(format!("http://127.0.0.1:{}/new-section/", port))
        .send()
        .expect("Failed to fetch new section");
    assert_eq!(resp.status().as_u16(), 404, "New section should not exist initially");

    // Create the new section directory and _index.md
    std::fs::create_dir_all(&new_section_dir).expect("Failed to create section dir");
    let section_content = r#"+++
title = "New Section"
+++

This is a dynamically created section.
"#;
    std::fs::write(&new_section_path, section_content).expect("Failed to create section index");

    // Wait for file watcher to pick up the change
    // Note: New directories may need more time for the watcher to detect
    std::thread::sleep(Duration::from_millis(1000));

    // Verify the new section is now accessible
    let resp = client
        .get(format!("http://127.0.0.1:{}/new-section/", port))
        .send()
        .expect("Failed to fetch new section after creation");

    assert!(
        resp.status().is_success(),
        "New section should be accessible after creation, got status {}",
        resp.status()
    );

    let body = resp.text().expect("Failed to read body");
    assert!(
        body.contains("dynamically created section"),
        "New section content should be present. Body:\n{body}"
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&new_section_dir);

    // Server cleanup handled by Drop
}
