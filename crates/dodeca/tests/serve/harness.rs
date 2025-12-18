//! Test harness for dodeca serve integration tests
//!
//! Provides high-level APIs for testing the server without boilerplate.
//!
//! Uses Unix socket FD passing to hand the listening socket to the server process,
//! avoiding stdout parsing races and ensuring immediate readiness.
//!
//! # Environment Variables
//!
//! - `DODECA_BIN`: Path to the ddc binary (defaults to cargo-built binary)
//! - `DODECA_CELL_PATH`: Path to cell binaries (defaults to same dir as ddc)
//! - `DODECA_TEST_WRAPPER`: Optional wrapper script/command to run ddc under
//!   (e.g., "valgrind --leak-check=full" or "strace -f -o /tmp/trace.out")

#![allow(clippy::disallowed_methods)]

use async_send_fd::AsyncSendFd;
use regex::Regex;
use reqwest::blocking::Client;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UnixListener;
use tracing::{debug, error, info};

/// Get the path to the ddc binary
fn ddc_binary() -> PathBuf {
    std::env::var("DODECA_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_BIN_EXE_ddc")))
}

/// Get the path to the cell binaries directory
fn cell_path() -> Option<PathBuf> {
    std::env::var("DODECA_CELL_PATH").map(PathBuf::from).ok()
}

/// Get the wrapper command if specified (e.g., "valgrind --leak-check=full")
fn test_wrapper() -> Option<Vec<String>> {
    std::env::var("DODECA_TEST_WRAPPER")
        .ok()
        .map(|s| s.split_whitespace().map(String::from).collect())
}

/// A running test site with a server and isolated fixture directory
pub struct TestSite {
    child: Child,
    port: u16,
    fixture_dir: PathBuf,
    _temp_dir: tempfile::TempDir,
    client: Client,
    _unix_socket_dir: tempfile::TempDir,
    logs: Arc<Mutex<Vec<String>>>,
}

impl TestSite {
    /// Create a new test site from a fixture directory name
    pub fn new(fixture_name: &str) -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest_dir.join("tests/fixtures").join(fixture_name);
        Self::from_source(&src)
    }

    /// Create a new test site from a fixture with custom files written before server start.
    /// This is useful for tests that need custom templates or content that should be
    /// loaded at server startup time rather than triggering livereload.
    pub fn with_files(fixture_name: &str, files: &[(&str, &str)]) -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest_dir.join("tests/fixtures").join(fixture_name);
        Self::from_source_with_files(&src, files)
    }

    /// Create a new test site from an arbitrary source directory
    pub fn from_source(src: &Path) -> Self {
        Self::from_source_with_files(src, &[])
    }

    /// Create a new test site from an arbitrary source directory with custom files
    pub fn from_source_with_files(src: &Path, files: &[(&str, &str)]) -> Self {
        // Create isolated temp directory
        let temp_dir = tempfile::Builder::new()
            .prefix("dodeca-test-")
            .tempdir()
            .expect("create temp dir");

        let fixture_dir = temp_dir.path().to_path_buf();
        copy_dir_recursive(src, &fixture_dir).expect("copy fixture");

        // Write any custom files before starting the server
        for (rel_path, content) in files {
            let path = fixture_dir.join(rel_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dir for custom file");
            }
            fs::write(&path, content)
                .unwrap_or_else(|e| panic!("write custom file {}: {e}", path.display()));
        }

        // Ensure .cache exists and is empty
        let cache_dir = fixture_dir.join(".cache");
        let _ = fs::remove_dir_all(&cache_dir);
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // Create Unix socket directory
        let unix_socket_dir = tempfile::Builder::new()
            .prefix("dodeca-sock-")
            .tempdir()
            .expect("create unix socket dir");

        let unix_socket_path = unix_socket_dir.path().join("server.sock");

        // Create runtime for async socket operations
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");

        // Bind TCP socket on port 0 (OS assigns port) using std (not tokio)
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind TCP");
        let port = std_listener.local_addr().expect("get local addr").port();
        info!(port, "Bound ephemeral TCP listener for test server");

        // Create Unix socket listener
        let unix_listener =
            rt.block_on(async { UnixListener::bind(&unix_socket_path).expect("bind Unix socket") });

        // Start server with Unix socket path
        let fixture_str = fixture_dir.to_string_lossy().to_string();
        let unix_socket_str = unix_socket_path.to_string_lossy().to_string();
        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "debug".to_string());

        let ddc = ddc_binary();
        let ddc_args = [
            "serve",
            &fixture_str,
            "--no-tui",
            "--fd-socket",
            &unix_socket_str,
        ];

        // Build command, optionally wrapping with DODECA_TEST_WRAPPER
        let mut cmd = if let Some(wrapper_parts) = test_wrapper() {
            let (wrapper_cmd, wrapper_args) = wrapper_parts
                .split_first()
                .expect("DODECA_TEST_WRAPPER must not be empty");
            let mut cmd = Command::new(wrapper_cmd);
            cmd.args(wrapper_args);
            cmd.arg(&ddc);
            cmd.args(ddc_args);
            info!(wrapper = %wrapper_cmd, "Using test wrapper");
            cmd
        } else {
            let mut cmd = Command::new(&ddc);
            cmd.args(ddc_args);
            cmd
        };

        cmd.env("RUST_LOG", &rust_log)
            .env("RUST_BACKTRACE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set cell path if provided via env var
        if let Some(cell_dir) = cell_path() {
            cmd.env("DODECA_CELL_PATH", &cell_dir);
        }

        let mut child = cmd.spawn().expect("start server");
        info!(child_pid = child.id(), ddc = %ddc.display(), %rust_log, "Spawned ddc server process");

        // Take stdout/stderr before the async block
        let stdout = child.stdout.take().expect("capture stdout");
        let stderr = child.stderr.take().expect("capture stderr");

        // Accept connection from server on Unix socket and send FD
        let child_id = child.id();
        rt.block_on(async {
            let accept_future = unix_listener.accept();
            let timeout_duration = tokio::time::Duration::from_secs(5);

            let (unix_stream, _) = tokio::time::timeout(timeout_duration, accept_future)
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "Timeout waiting for server (PID {}) to connect to Unix socket within 5s",
                        child_id
                    )
                })
                .expect("Failed to accept Unix connection");

            // Send the TCP listener FD to the server
            unix_stream
                .send_fd(std_listener.as_raw_fd())
                .await
                .expect("send FD");
            info!("Sent TCP listener FD to server");

            // IMPORTANT: Don't drop std_listener here - keep it alive!
            // The FD is shared with the server now
            std::mem::forget(std_listener);
        });

        // Wait for server to signal readiness (it prints to stdout after receiving FD)
        // Use a timeout to avoid hanging forever
        use std::io::Read;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut stderr_reader = BufReader::new(stderr);

        // Try to read with a timeout by doing a non-blocking read loop
        let start = Instant::now();
        let timeout_dur = Duration::from_secs(10);
        loop {
            if start.elapsed() > timeout_dur {
                let mut stderr_content = String::new();
                let _ = stderr_reader.read_to_string(&mut stderr_content);
                error!(stderr = %stderr_content, "Timeout waiting for READY signal");
                panic!("Timeout waiting for READY signal after {:?}", timeout_dur);
            }

            // Try to read a line
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // EOF - server closed stdout
                    let mut stderr_content = String::new();
                    let _ = stderr_reader.read_to_string(&mut stderr_content);
                    error!("Server closed stdout before printing READY");
                    error!(stderr = %stderr_content, "Server stderr on early EOF");
                    panic!("Server closed stdout before printing READY");
                }
                Ok(_) => {
                    // Got a line
                    if line.contains("READY") {
                        info!("Received READY from server");
                        break; // Success!
                    } else {
                        debug!(line = line.trim_end(), "Server stdout");
                        line.clear();
                        continue;
                    }
                }
                Err(e) => {
                    let mut stderr_content = String::new();
                    let _ = stderr_reader.read_to_string(&mut stderr_content);
                    error!(error = %e, stderr = %stderr_content, "Failed to read READY signal");
                    panic!("Failed to read READY signal: {}", e);
                }
            }
        }

        // Capture logs (stdout/stderr) for assertions + also forward to terminal for debugging.
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // Drain stdout in background - use println! to preserve ANSI escape codes
        // (tracing-subscriber's DefaultVisitor wraps messages in Escape() which
        // converts \x1b bytes to literal "\\x1b" strings as a security measure
        // against terminal injection attacks - there's no config to disable it,
        // you'd need a custom FormatEvent implementation)
        let logs_stdout = Arc::clone(&logs);
        std::thread::spawn(move || {
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        logs_stdout.lock().unwrap().push(l.clone());
                        println!("{l}");
                    }
                    Err(e) => eprintln!("Failed to read server stdout: {e}"),
                }
            }
        });

        // Drain stderr in background
        let logs_stderr = Arc::clone(&logs);
        std::thread::spawn(move || {
            for line in stderr_reader.lines() {
                match line {
                    Ok(l) => {
                        logs_stderr.lock().unwrap().push(l.clone());
                        eprintln!("{l}");
                    }
                    Err(e) => eprintln!("Failed to read server stderr: {e}"),
                }
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build http client");

        Self {
            child,
            port,
            fixture_dir,
            _temp_dir: temp_dir,
            client,
            _unix_socket_dir: unix_socket_dir,
            logs,
        }
    }

    /// Clear captured logs (stdout + stderr).
    pub fn clear_logs(&self) {
        self.logs.lock().unwrap().clear();
    }

    /// Return the current log cursor (index into the captured log vector).
    pub fn log_cursor(&self) -> usize {
        self.logs.lock().unwrap().len()
    }

    /// Count log lines containing `needle` since `cursor`.
    pub fn count_logs_since(&self, cursor: usize, needle: &str) -> usize {
        let logs = self.logs.lock().unwrap();
        logs.iter()
            .skip(cursor)
            .filter(|l| l.contains(needle))
            .count()
    }

    /// Make a GET request to a path
    pub fn get(&self, path: &str) -> Response {
        let url = format!("http://127.0.0.1:{}{}", self.port, path);
        debug!(%url, "Issuing GET request");

        match self.client.get(&url).send() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().unwrap_or_default();
                debug!(%url, status, "Received response");
                Response { status, body, url }
            }
            Err(e) => {
                error!(%url, error = %e, "GET failed");
                panic!("GET {} failed: {}", url, e);
            }
        }
    }

    /// Wait for a path to return 200, retrying until timeout
    pub fn wait_for(&self, path: &str, timeout: Duration) -> Response {
        let deadline = Instant::now() + timeout;

        loop {
            let resp = self.get(path);
            if resp.status == 200 {
                return resp;
            }

            if Instant::now() >= deadline {
                panic!(
                    "Path {} did not return 200 within {:?} (last status: {})",
                    path, timeout, resp.status
                );
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Wait until a condition is true, retrying until timeout
    /// Returns the value produced by the condition, or panics on timeout
    pub fn wait_until<T, F>(&self, timeout: Duration, mut condition: F) -> T
    where
        F: FnMut() -> Option<T>,
    {
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(value) = condition() {
                return value;
            }

            if Instant::now() >= deadline {
                break;
            }

            std::thread::sleep(Duration::from_millis(100));
        }

        panic!("Condition not met within {:?}", timeout);
    }

    /// Get the fixture directory path
    #[allow(dead_code)]
    pub fn fixture_dir(&self) -> &Path {
        &self.fixture_dir
    }

    /// Read a file from the fixture directory
    pub fn read_file(&self, rel_path: &str) -> String {
        let path = self.fixture_dir.join(rel_path);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Write a file to the fixture directory
    pub fn write_file(&self, rel_path: &str, content: &str) {
        let path = self.fixture_dir.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, content).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }

    /// Modify a file in place
    pub fn modify_file<F>(&self, rel_path: &str, f: F)
    where
        F: FnOnce(&str) -> String,
    {
        let content = self.read_file(rel_path);
        let modified = f(&content);
        self.write_file(rel_path, &modified);
    }

    /// Delete a file from the fixture directory
    pub fn delete_file(&self, rel_path: &str) {
        let path = self.fixture_dir.join(rel_path);
        fs::remove_file(&path).unwrap_or_else(|e| panic!("delete {}: {e}", path.display()));
    }

    /// Delete a file or directory from the fixture directory, ignoring if it doesn't exist
    pub fn delete_if_exists(&self, rel_path: &str) {
        let path = self.fixture_dir.join(rel_path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&path);
    }

    /// Wait for the file watcher debounce window
    pub fn wait_debounce(&self) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

impl Drop for TestSite {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An HTTP response
pub struct Response {
    pub status: u16,
    pub body: String,
    pub url: String,
}

impl Response {
    /// Assert the response is 200 OK
    pub fn assert_ok(&self) {
        assert_eq!(
            self.status, 200,
            "Expected 200 OK for {}, got {}",
            self.url, self.status
        );
    }

    /// Assert the response body contains a substring
    pub fn assert_contains(&self, needle: &str) {
        assert!(
            self.body.contains(needle),
            "Response body for {} does not contain '{}'\nActual body (first 500 chars): {}",
            self.url,
            needle,
            &self.body[..self.body.len().min(500)]
        );
    }

    /// Assert the response body does NOT contain a substring
    pub fn assert_not_contains(&self, needle: &str) {
        assert!(
            !self.body.contains(needle),
            "Response body for {} should not contain '{}', but it does",
            self.url,
            needle
        );
    }

    /// Get the body text
    pub fn text(&self) -> &str {
        &self.body
    }

    /// Find an <img> tag's src attribute matching a glob pattern
    /// Returns the matched src value (without host) or None
    pub fn img_src(&self, pattern: &str) -> Option<String> {
        // Convert glob pattern to regex (non-greedy to avoid capturing too much)
        let pattern_re = pattern.replace(".", r"\.").replace("*", "[^\"]*?");
        let re = Regex::new(&format!(r#"<img[^>]+src="({}[^"]*)""#, pattern_re)).ok()?;

        re.captures(&self.body)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Find a <link> tag's href attribute matching a glob pattern
    /// Returns the matched href value (without host) or None
    pub fn css_link(&self, pattern: &str) -> Option<String> {
        // Convert glob pattern to regex (non-greedy to avoid capturing too much)
        let pattern_re = pattern.replace(".", r"\.").replace("*", "[^\"]*?");
        let re = Regex::new(&format!(r#"<link[^>]+href="({}[^"]*)""#, pattern_re)).ok()?;

        re.captures(&self.body)
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Extract a value using a regex with one capture group
    pub fn extract(&self, pattern: &str) -> Option<String> {
        let re = Regex::new(pattern).expect("valid regex");
        re.captures(&self.body)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
    }

    /// Extract a value using a regex, panic if not found
    #[allow(dead_code)]
    pub fn extract_or_panic(&self, pattern: &str) -> String {
        self.extract(pattern)
            .unwrap_or_else(|| panic!("Pattern '{pattern}' not found in response"))
    }
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// Result of running `ddc build` on a fixture
pub struct BuildResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl BuildResult {
    /// Assert the build succeeded
    pub fn assert_success(&self) -> &Self {
        assert!(
            self.success,
            "Build should have succeeded but failed.\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    /// Assert the build failed
    #[allow(dead_code)]
    pub fn assert_failure(&self) -> &Self {
        assert!(
            !self.success,
            "Build should have failed but succeeded.\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    /// Assert the build output (stdout + stderr) contains a string
    pub fn assert_output_contains(&self, needle: &str) -> &Self {
        let combined = format!("{}{}", self.stdout, self.stderr);
        assert!(
            combined.contains(needle),
            "Build output should contain '{}' but doesn't.\nstdout:\n{}\nstderr:\n{}",
            needle,
            self.stdout,
            self.stderr
        );
        self
    }
}

/// Build a site from an arbitrary source directory
fn build_site_from_source(src: &Path) -> BuildResult {
    // Create isolated temp directory
    let temp_dir = tempfile::Builder::new()
        .prefix("dodeca-build-test-")
        .tempdir()
        .expect("create temp dir");

    let fixture_dir = temp_dir.path().to_path_buf();
    copy_dir_recursive(src, &fixture_dir).expect("copy fixture");

    // Ensure .cache exists and is empty
    let cache_dir = fixture_dir.join(".cache");
    let _ = fs::remove_dir_all(&cache_dir);
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    // Create output directory
    let output_dir = fixture_dir.join("public");
    fs::create_dir_all(&output_dir).expect("create output dir");

    // Run build
    let fixture_str = fixture_dir.to_string_lossy().to_string();
    let ddc = ddc_binary();
    let mut cmd = Command::new(&ddc);
    cmd.args(["build", &fixture_str]);

    // Set cell path if provided via env var
    if let Some(cell_dir) = cell_path() {
        cmd.env("DODECA_CELL_PATH", &cell_dir);
    }

    let output = cmd.output().expect("run build");

    BuildResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Helper for creating test sites from inline content
pub struct InlineSite {
    _temp_dir: tempfile::TempDir,
    pub fixture_dir: PathBuf,
}

impl InlineSite {
    /// Create a new inline site with the given markdown content
    pub fn new(content_files: &[(&str, &str)]) -> Self {
        let temp_dir = tempfile::Builder::new()
            .prefix("dodeca-inline-test-")
            .tempdir()
            .expect("create temp dir");

        let fixture_dir = temp_dir.path().to_path_buf();

        // Create directories
        fs::create_dir_all(fixture_dir.join("content")).expect("create content dir");
        fs::create_dir_all(fixture_dir.join("templates")).expect("create templates dir");
        fs::create_dir_all(fixture_dir.join("sass")).expect("create sass dir");
        fs::create_dir_all(fixture_dir.join(".config")).expect("create config dir");
        fs::create_dir_all(fixture_dir.join(".cache")).expect("create cache dir");

        // Write config
        fs::write(
            fixture_dir.join(".config/dodeca.kdl"),
            "content \"content\"\noutput \"public\"\n",
        )
        .expect("write config");

        // Write templates
        fs::write(
            fixture_dir.join("templates/index.html"),
            "<!DOCTYPE html><html><head><title>{{ section.title }}</title></head><body>{{ section.content | safe }}</body></html>",
        )
        .expect("write index template");

        fs::write(
            fixture_dir.join("templates/section.html"),
            "<!DOCTYPE html><html><head><title>{{ section.title }}</title></head><body>{{ section.content | safe }}</body></html>",
        )
        .expect("write section template");

        fs::write(
            fixture_dir.join("templates/page.html"),
            "<!DOCTYPE html><html><head><title>{{ page.title }}</title></head><body>{{ page.content | safe }}</body></html>",
        )
        .expect("write page template");

        // Write sass
        fs::write(fixture_dir.join("sass/main.scss"), "body { margin: 0; }").expect("write sass");

        // Write content files
        for (path, content) in content_files {
            let file_path = fixture_dir.join("content").join(path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).expect("create content parent dir");
            }
            fs::write(&file_path, content).expect("write content file");
        }

        Self {
            _temp_dir: temp_dir,
            fixture_dir,
        }
    }

    /// Build this site
    pub fn build(&self) -> BuildResult {
        build_site_from_source(&self.fixture_dir)
    }
}
