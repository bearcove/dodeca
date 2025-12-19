//! Test harness for dodeca serve integration tests
//!
//! Provides high-level APIs for testing the server without boilerplate.
//!
//! Uses Unix socket FD passing to hand the listening socket to the server process.
//! The test harness binds the socket first, so connections queue in the TCP backlog
//! until the server is ready to accept.
//!
//! # Environment Variables
//!
//! - `DODECA_BIN`: Path to the ddc binary (required)
//! - `DODECA_CELL_PATH`: Path to cell binaries (defaults to same dir as ddc)
//! - `DODECA_TEST_WRAPPER`: Optional wrapper script/command to run ddc under
//!   (e.g., "valgrind --leak-check=full" or "strace -f -o /tmp/trace.out")

use async_send_fd::AsyncSendFd;
use regex::Regex;
use reqwest::blocking::Client;
use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UnixListener;
use tracing::{debug, error};

// Thread-local storage for logs from the last test (for printing on failure)
thread_local! {
    static LAST_TEST_LOGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static LAST_TEST_SETUP: RefCell<Option<Duration>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct LogLine {
    ts: Duration,
    line: String,
}

/// Get the logs from the last test that ran (for printing on failure)
pub fn get_last_test_logs() -> Vec<String> {
    LAST_TEST_LOGS.with(|logs| logs.borrow().clone())
}

/// Clear the last test logs
pub fn clear_last_test_logs() {
    LAST_TEST_LOGS.with(|logs| logs.borrow_mut().clear());
}

/// Clear the last test setup duration
pub fn clear_last_test_setup() {
    LAST_TEST_SETUP.with(|setup| *setup.borrow_mut() = None);
}

/// Get the setup duration from the last test site creation
pub fn get_last_test_setup() -> Option<Duration> {
    LAST_TEST_SETUP.with(|setup| *setup.borrow())
}

/// Get the path to the ddc binary
fn ddc_binary() -> PathBuf {
    std::env::var("DODECA_BIN")
        .map(PathBuf::from)
        .expect("DODECA_BIN environment variable must be set")
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
    logs: Arc<Mutex<Vec<LogLine>>>,
}

impl TestSite {
    /// Create a new test site from a fixture directory name
    pub fn new(fixture_name: &str) -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest_dir.join("fixtures").join(fixture_name);
        Self::from_source(&src)
    }

    /// Create a new test site from a fixture with custom files written before server start.
    /// This is useful for tests that need custom templates or content that should be
    /// loaded at server startup time rather than triggering livereload.
    pub fn with_files(fixture_name: &str, files: &[(&str, &str)]) -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest_dir.join("fixtures").join(fixture_name);
        Self::from_source_with_files(&src, files)
    }

    /// Create a new test site from an arbitrary source directory
    pub fn from_source(src: &Path) -> Self {
        Self::from_source_with_files(src, &[])
    }

    /// Create a new test site from an arbitrary source directory with custom files
    pub fn from_source_with_files(src: &Path, files: &[(&str, &str)]) -> Self {
        let setup_start = Instant::now();
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
        debug!(port, "Bound ephemeral TCP listener for test server");

        // Create Unix socket listener
        let unix_listener =
            rt.block_on(async { UnixListener::bind(&unix_socket_path).expect("bind Unix socket") });

        // Start server with Unix socket path
        let fixture_str = fixture_dir.to_string_lossy().to_string();
        let unix_socket_str = unix_socket_path.to_string_lossy().to_string();
        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

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
            let mut cmd = StdCommand::new(wrapper_cmd);
            cmd.args(wrapper_args);
            cmd.arg(&ddc);
            cmd.args(ddc_args);
            debug!(wrapper = %wrapper_cmd, "Using test wrapper");
            cmd
        } else {
            let mut cmd = StdCommand::new(&ddc);
            cmd.args(ddc_args);
            cmd
        };

        cmd.env("RUST_LOG", &rust_log)
            .env("RUST_BACKTRACE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Keep code-exec builds fast and isolated per test (avoids cross-test contention under
        // nextest parallelism).
        let code_exec_target_dir = temp_dir.path().join("code-exec-target");
        let _ = std::fs::create_dir_all(&code_exec_target_dir);
        cmd.env("DDC_CODE_EXEC_TARGET_DIR", &code_exec_target_dir);

        // Set cell path if provided via env var
        if let Some(cell_dir) = cell_path() {
            cmd.env("DODECA_CELL_PATH", &cell_dir);
        }

        // Enable death-watch so ddc (and its cells) die when the test process dies.
        // This prevents orphan accumulation when tests are killed or crash.
        cmd.env("DODECA_DIE_WITH_PARENT", "1");

        let mut child = ur_taking_me_with_you::spawn_dying_with_parent(cmd)
            .expect("start server with death-watch");
        debug!(child_pid = child.id(), ddc = %ddc.display(), %rust_log, "Spawned ddc server process");

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
            debug!("Sent TCP listener FD to server");

            // IMPORTANT: Don't drop std_listener here - keep it alive!
            // The FD is shared with the server now
            std::mem::forget(std_listener);
        });

        // Capture logs (stdout/stderr) for assertions. Only printed on test failure.
        let reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);
        let logs: Arc<Mutex<Vec<LogLine>>> = Arc::new(Mutex::new(Vec::new()));
        let log_start = Instant::now();

        // Drain stdout in background (capture only, no printing)
        let logs_stdout = Arc::clone(&logs);
        let log_start_stdout = log_start;
        std::thread::spawn(move || {
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let entry = LogLine {
                            ts: log_start_stdout.elapsed(),
                            line: format!("[stdout] {l}"),
                        };
                        logs_stdout.lock().unwrap().push(entry);
                    }
                    Err(_) => break,
                }
            }
        });

        // Drain stderr in background (capture only, no printing)
        let logs_stderr = Arc::clone(&logs);
        let log_start_stderr = log_start;
        std::thread::spawn(move || {
            for line in stderr_reader.lines() {
                match line {
                    Ok(l) => {
                        let entry = LogLine {
                            ts: log_start_stderr.elapsed(),
                            line: format!("[stderr] {l}"),
                        };
                        logs_stderr.lock().unwrap().push(entry);
                    }
                    Err(_) => break,
                }
            }
        });

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build http client");

        let setup_elapsed = setup_start.elapsed();
        LAST_TEST_SETUP.with(|setup| {
            *setup.borrow_mut() = Some(setup_elapsed);
        });

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
            .filter(|l| l.line.contains(needle))
            .count()
    }

    /// Make a GET request to a path
    pub fn get(&self, path: &str) -> Response {
        let url = format!("http://127.0.0.1:{}{}", self.port, path);
        debug!(%url, "Issuing GET request");

        fn format_error_chain(err: &dyn std::error::Error) -> String {
            let mut out = err.to_string();
            let mut cur = err.source();
            while let Some(e) = cur {
                out.push_str("\n  caused by: ");
                out.push_str(&e.to_string());
                cur = e.source();
            }
            out
        }

        // Server should only accept connections when it's ready to serve them.
        // No retries - if the request fails, it's a real failure.
        match self.client.get(&url).send() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().unwrap_or_default();
                debug!(%url, status, "Received response");
                Response { status, body, url }
            }
            Err(e) => {
                error!(%url, error = ?e, "GET failed");
                panic!("GET {} failed:\n{:?}\n{}", url, e, format_error_chain(&e));
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
        // Save logs to thread-local for potential failure reporting
        let logs = render_logs(&self.logs);
        LAST_TEST_LOGS.with(|tl| {
            *tl.borrow_mut() = logs;
        });

        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn render_logs(logs: &Arc<Mutex<Vec<LogLine>>>) -> Vec<String> {
    let mut lines = logs.lock().unwrap().clone();
    lines.sort_by_key(|l| l.ts);
    lines
        .into_iter()
        .map(|l| format!("{:>8.3}s {}", l.ts.as_secs_f64(), l.line))
        .collect()
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

// ============================================================================
// BUILD TESTS (for code execution tests)
// ============================================================================

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

    /// Build this site (sync version for standalone test runner)
    pub fn build(&self) -> BuildResult {
        build_site_from_source_sync(&self.fixture_dir)
    }
}

/// Build a site from an arbitrary source directory (sync version)
fn build_site_from_source_sync(src: &Path) -> BuildResult {
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
    let mut cmd = StdCommand::new(&ddc);
    cmd.args(["build", &fixture_str]);

    // Set cell path if provided via env var
    if let Some(cell_dir) = cell_path() {
        cmd.env("DODECA_CELL_PATH", &cell_dir);
    }

    // Isolate code-execution build artifacts per build invocation to avoid macOS hangs due to
    // cargo file-lock contention under concurrent tests/processes.
    let code_exec_target_dir = temp_dir.path().join("code-exec-target");
    let _ = fs::create_dir_all(&code_exec_target_dir);
    cmd.env("DDC_CODE_EXEC_TARGET_DIR", &code_exec_target_dir);

    let output = cmd.output().expect("run build");

    BuildResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}
