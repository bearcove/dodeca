//! Test harness for dodeca serve integration tests
//!
//! Provides high-level APIs for testing the server without boilerplate.
//!
//! Uses Unix socket FD passing to hand the listening socket to the acceptor process.
//! The test harness binds the socket first, so connections queue in the TCP backlog
//! until the acceptor starts accepting.
//!
//! # Environment Variables
//!
//! - `DODECA_BIN`: Path to the ddc binary (required)
//! - `DODECA_CELL_PATH`: Path to cell binaries (defaults to same dir as ddc)
//! - `DODECA_TEST_WRAPPER`: Optional wrapper script/command to run ddc under
//!   (e.g., "valgrind --leak-check=full" or "strace -f -o /tmp/trace.out")

use async_send_fd::AsyncSendFd;
use regex::Regex;
use std::cell::Cell;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UnixListener;
use tracing::{debug, error};

// Thread-local storage for the active test id (used to route logs).
thread_local! {
    static CURRENT_TEST_ID: Cell<u64> = const { Cell::new(0) };
}

static TEST_LOGS: OnceLock<Mutex<std::collections::HashMap<u64, Vec<LogLine>>>> = OnceLock::new();
static TEST_EXIT_STATUS: OnceLock<Mutex<std::collections::HashMap<u64, std::process::ExitStatus>>> =
    OnceLock::new();
static TEST_SETUP: OnceLock<Mutex<std::collections::HashMap<u64, Duration>>> = OnceLock::new();

#[derive(Clone)]
struct LogLine {
    ts: Duration,
    abs: SystemTime,
    line: String,
}

fn push_log(logs: &Arc<Mutex<Vec<LogLine>>>, log_start: Instant, line: impl Into<String>) {
    let entry = LogLine {
        ts: log_start.elapsed(),
        abs: SystemTime::now(),
        line: line.into(),
    };
    logs.lock().unwrap().push(entry);
}

/// Set the current test id for log routing
pub fn set_current_test_id(id: u64) {
    CURRENT_TEST_ID.with(|cell| cell.set(id));
}

/// Clear per-test logs and setup timing
pub fn clear_test_state(id: u64) {
    let logs = TEST_LOGS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let setup = TEST_SETUP.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    logs.lock().unwrap().remove(&id);
    setup.lock().unwrap().remove(&id);
}

/// Get the logs for a test id (for printing on failure)
pub fn get_logs_for(id: u64) -> Vec<String> {
    let logs = TEST_LOGS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let lines = logs.lock().unwrap().get(&id).cloned().unwrap_or_default();
    render_logs(lines)
}

/// Get the exit status for a test id (if the server already exited)
pub fn get_exit_status_for(id: u64) -> Option<std::process::ExitStatus> {
    let statuses = TEST_EXIT_STATUS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    statuses.lock().unwrap().get(&id).copied()
}

/// Get the setup duration for a test id
pub fn get_setup_for(id: u64) -> Option<Duration> {
    let setup = TEST_SETUP.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    setup.lock().unwrap().get(&id).copied()
}

/// Get the path to the ddc binary
fn ddc_binary() -> PathBuf {
    std::env::var("DODECA_BIN")
        .map(PathBuf::from)
        .expect("DODECA_BIN environment variable must be set")
}

/// Get the path to the acceptor binary
fn acceptor_binary() -> PathBuf {
    if let Ok(path) = std::env::var("DODECA_ACCEPTOR_BIN") {
        return PathBuf::from(path);
    }

    let ddc = ddc_binary();
    let parent = ddc.parent().expect("DODECA_BIN must have a parent dir");
    let acceptor = parent.join("ddc-acceptor");
    if !acceptor.exists() {
        panic!(
            "ddc-acceptor binary not found at {} (set DODECA_ACCEPTOR_BIN to override)",
            acceptor.display()
        );
    }
    acceptor
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
    acceptor_child: Option<Child>,
    port: u16,
    fixture_dir: PathBuf,
    _temp_dir: tempfile::TempDir,
    _unix_socket_dir: tempfile::TempDir,
    logs: Arc<Mutex<Vec<LogLine>>>,
    log_start: Instant,
    test_id: u64,
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
        let test_id = CURRENT_TEST_ID.with(|cell| cell.get());
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

        let fd_socket_path = unix_socket_dir.path().join("fd.sock");
        let acceptor_socket_path = unix_socket_dir.path().join("acceptor.sock");

        // Create runtime for async socket operations
        let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");

        // Bind TCP socket on port 0 (OS assigns port) using std (not tokio)
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind TCP");
        let port = std_listener.local_addr().expect("get local addr").port();
        debug!(port, "Bound ephemeral TCP listener for test server");

        // Create Unix socket listener
        let unix_listener =
            rt.block_on(async { UnixListener::bind(&fd_socket_path).expect("bind Unix socket") });

        // Start server with Unix socket path
        let fixture_str = fixture_dir.to_string_lossy().to_string();
        let fd_socket_str = fd_socket_path.to_string_lossy().to_string();
        let acceptor_socket_str = acceptor_socket_path.to_string_lossy().to_string();
        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

        // Start acceptor with Unix socket paths
        let acceptor = acceptor_binary();
        let mut acceptor_cmd = StdCommand::new(&acceptor);
        acceptor_cmd
            .arg("--fd-socket")
            .arg(&fd_socket_str)
            .arg("--acceptor-socket")
            .arg(&acceptor_socket_str)
            .env("RUST_LOG", &rust_log)
            .env("RUST_BACKTRACE", "1")
            .env("DDC_LOG_TIME", "utc")
            .env("DODECA_DIE_WITH_PARENT", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut acceptor_child = ur_taking_me_with_you::spawn_dying_with_parent(acceptor_cmd)
            .expect("start acceptor process");
        debug!(
            acceptor_pid = acceptor_child.id(),
            acceptor = %acceptor.display(),
            "Spawned acceptor process"
        );

        let ddc = ddc_binary();
        let port_string = port.to_string();
        let ddc_args = [
            "serve",
            &fixture_str,
            "--no-tui",
            "--acceptor-socket",
            &acceptor_socket_str,
            "--port",
            &port_string,
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
            .env("DDC_LOG_TIME", "utc")
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
        let acceptor_stdout = acceptor_child
            .stdout
            .take()
            .expect("capture acceptor stdout");
        let acceptor_stderr = acceptor_child
            .stderr
            .take()
            .expect("capture acceptor stderr");

        // Capture logs (stdout/stderr + harness events). Only printed on test failure.
        let reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);
        let logs: Arc<Mutex<Vec<LogLine>>> = Arc::new(Mutex::new(Vec::new()));
        let log_start = Instant::now();
        push_log(
            &logs,
            log_start,
            format!("[harness] spawned ddc pid={}", child.id()),
        );
        push_log(
            &logs,
            log_start,
            format!("[harness] spawned acceptor pid={}", acceptor_child.id()),
        );

        // Accept connection from acceptor on Unix socket and send FD
        let child_id = child.id();
        let logs_for_fd = Arc::clone(&logs);
        rt.block_on(async {
            push_log(
                &logs_for_fd,
                log_start,
                "[harness] waiting for acceptor unix socket accept",
            );
            let accept_future = unix_listener.accept();
            let timeout_duration = tokio::time::Duration::from_secs(5);

            let (unix_stream, _) = tokio::time::timeout(timeout_duration, accept_future)
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "Timeout waiting for acceptor to connect to Unix socket within 5s (server PID {})",
                        child_id
                    )
                })
                .expect("Failed to accept Unix connection");

            // Send the TCP listener FD to the acceptor
            unix_stream
                .send_fd(std_listener.as_raw_fd())
                .await
                .expect("send FD");
            push_log(
                &logs_for_fd,
                log_start,
                "[harness] sent TCP listener FD to acceptor",
            );
            debug!("Sent TCP listener FD to acceptor");

            // IMPORTANT: Don't drop std_listener here - keep it alive!
            // The FD is shared with the acceptor now
            std::mem::forget(std_listener);
        });
        push_log(&logs, log_start, "[harness] log capture started");

        // Drain stdout in background (capture only, no printing)
        let logs_stdout = Arc::clone(&logs);
        let log_start_stdout = log_start;
        std::thread::spawn(move || {
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        push_log(&logs_stdout, log_start_stdout, format!("[stdout] {l}"));
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
                        push_log(&logs_stderr, log_start_stderr, format!("[stderr] {l}"));
                    }
                    Err(_) => break,
                }
            }
        });

        // Drain acceptor stdout in background
        let acceptor_reader = BufReader::new(acceptor_stdout);
        let logs_acceptor_stdout = Arc::clone(&logs);
        let log_start_acceptor_stdout = log_start;
        std::thread::spawn(move || {
            for line in acceptor_reader.lines() {
                match line {
                    Ok(l) => {
                        push_log(
                            &logs_acceptor_stdout,
                            log_start_acceptor_stdout,
                            format!("[acceptor stdout] {l}"),
                        );
                    }
                    Err(_) => break,
                }
            }
        });

        // Drain acceptor stderr in background
        let acceptor_stderr_reader = BufReader::new(acceptor_stderr);
        let logs_acceptor_stderr = Arc::clone(&logs);
        let log_start_acceptor_stderr = log_start;
        std::thread::spawn(move || {
            for line in acceptor_stderr_reader.lines() {
                match line {
                    Ok(l) => {
                        push_log(
                            &logs_acceptor_stderr,
                            log_start_acceptor_stderr,
                            format!("[acceptor stderr] {l}"),
                        );
                    }
                    Err(_) => break,
                }
            }
        });

        let setup_elapsed = setup_start.elapsed();
        let setup = TEST_SETUP.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
        setup.lock().unwrap().insert(test_id, setup_elapsed);

        Self {
            child,
            acceptor_child: Some(acceptor_child),
            port,
            fixture_dir,
            _temp_dir: temp_dir,
            _unix_socket_dir: unix_socket_dir,
            logs,
            log_start,
            test_id,
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
        push_log(&self.logs, self.log_start, format!("[harness] GET {url}"));

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

        // Retry connection reset errors (macOS flakiness).
        fn is_connection_reset(err: &ureq::Error) -> bool {
            match err {
                ureq::Error::Io(io) => {
                    io.kind() == std::io::ErrorKind::ConnectionReset
                        || io.to_string().contains("Connection reset by peer")
                        || io.to_string().contains("os error 54")
                }
                _ => false,
            }
        }

        let max_retries = std::env::var("DODECA_RETRIES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(5);
        let max_retries = max_retries.max(1);
        for attempt in 0..max_retries {
            match ureq::get(&url).call() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = resp.into_body().read_to_string().unwrap_or_default();
                    debug!(%url, status, "Received response");
                    return Response { status, body, url };
                }
                Err(ureq::Error::StatusCode(status)) => {
                    // ureq treats 4xx/5xx as errors, but we want to return them as responses
                    let body = String::new();
                    debug!(%url, status, "Received error status");
                    return Response { status, body, url };
                }
                Err(e) => {
                    push_log(
                        &self.logs,
                        self.log_start,
                        format!(
                            "[harness] GET error attempt {}/{}: {}",
                            attempt + 1,
                            max_retries,
                            e
                        ),
                    );
                    if is_connection_reset(&e) && attempt + 1 < max_retries {
                        push_log(
                            &self.logs,
                            self.log_start,
                            format!(
                                "[harness] retrying after connection reset attempt {}/{}",
                                attempt + 1,
                                max_retries
                            ),
                        );
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }

                    error!(%url, error = ?e, "GET failed");
                    panic!("GET {} failed:\n{:?}\n{}", url, e, format_error_chain(&e));
                }
            }
        }

        unreachable!("retry loop should always return or panic");
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
        push_log(
            &self.logs,
            self.log_start,
            format!("[harness] drop: cleaning up ddc pid={}", self.child.id()),
        );

        match self.child.try_wait() {
            Ok(Some(status)) => {
                push_log(
                    &self.logs,
                    self.log_start,
                    format!("[harness] drop: ddc already exited status={status}"),
                );
                let statuses =
                    TEST_EXIT_STATUS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
                statuses.lock().unwrap().insert(self.test_id, status);
            }
            Ok(None) => {
                push_log(&self.logs, self.log_start, "[harness] drop: killing ddc");
                if let Err(err) = self.child.kill() {
                    push_log(
                        &self.logs,
                        self.log_start,
                        format!("[harness] drop: kill failed: {err}"),
                    );
                }

                match self.child.wait() {
                    Ok(status) => {
                        push_log(
                            &self.logs,
                            self.log_start,
                            format!("[harness] drop: wait complete status={status}"),
                        );
                        let statuses = TEST_EXIT_STATUS
                            .get_or_init(|| Mutex::new(std::collections::HashMap::new()));
                        statuses.lock().unwrap().insert(self.test_id, status);
                    }
                    Err(err) => {
                        push_log(
                            &self.logs,
                            self.log_start,
                            format!("[harness] drop: wait failed: {err}"),
                        );
                    }
                }
            }
            Err(err) => {
                push_log(
                    &self.logs,
                    self.log_start,
                    format!("[harness] drop: try_wait failed: {err}"),
                );
            }
        }

        if let Some(mut acceptor_child) = self.acceptor_child.take() {
            push_log(
                &self.logs,
                self.log_start,
                format!(
                    "[harness] drop: cleaning up acceptor pid={}",
                    acceptor_child.id()
                ),
            );

            match acceptor_child.try_wait() {
                Ok(Some(status)) => {
                    push_log(
                        &self.logs,
                        self.log_start,
                        format!("[harness] drop: acceptor already exited status={status}"),
                    );
                }
                Ok(None) => {
                    push_log(
                        &self.logs,
                        self.log_start,
                        "[harness] drop: killing acceptor",
                    );
                    if let Err(err) = acceptor_child.kill() {
                        push_log(
                            &self.logs,
                            self.log_start,
                            format!("[harness] drop: acceptor kill failed: {err}"),
                        );
                    }

                    match acceptor_child.wait() {
                        Ok(status) => {
                            push_log(
                                &self.logs,
                                self.log_start,
                                format!("[harness] drop: acceptor wait complete status={status}"),
                            );
                        }
                        Err(err) => {
                            push_log(
                                &self.logs,
                                self.log_start,
                                format!("[harness] drop: acceptor wait failed: {err}"),
                            );
                        }
                    }
                }
                Err(err) => {
                    push_log(
                        &self.logs,
                        self.log_start,
                        format!("[harness] drop: acceptor try_wait failed: {err}"),
                    );
                }
            }
        }

        let logs = self.logs.lock().unwrap().clone();
        let logs_map = TEST_LOGS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
        logs_map.lock().unwrap().insert(self.test_id, logs);
    }
}

fn render_logs(mut lines: Vec<LogLine>) -> Vec<String> {
    lines.sort_by_key(|l| {
        l.abs
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
    });
    lines
        .into_iter()
        .map(|l| {
            let abs = l
                .abs
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|_| Duration::from_secs(0));
            format!(
                "{:>10}.{:03}Z {:>8.3}s {}",
                abs.as_secs(),
                abs.subsec_millis(),
                l.ts.as_secs_f64(),
                l.line
            )
        })
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
