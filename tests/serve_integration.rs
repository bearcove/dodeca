//! Integration tests for the serve functionality
//!
//! These tests verify that all pages are accessible when serving a site.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Start the server and return the child process
fn start_server(fixture_path: &str, port: u16) -> Child {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_full_path = format!("{}/tests/fixtures/{}", manifest_dir, fixture_path);

    Command::new(env!("CARGO_BIN_EXE_ddc"))
        .args(["serve", &fixture_full_path, "-p", &port.to_string(), "--no-tui"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start server")
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
#[test]
fn test_all_pages_accessible() {
    // Use a unique port to avoid conflicts
    let port = 14567;

    // Start the server
    let mut server = start_server("sample-site", port);

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
    server.kill().ok();
    server.wait().ok();

    // Report all failures at once
    if !failures.is_empty() {
        panic!("Page accessibility failures:\n{}", failures.join("\n"));
    }
}
