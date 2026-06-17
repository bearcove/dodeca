//! Test harness for dodeca serve integration tests
//!
//! Provides high-level APIs for testing the server without boilerplate.
//!
//! Uses FD/socket passing to hand the listening socket to the server process:
//! - **Unix**: Uses Unix domain sockets with SCM_RIGHTS
//! - **Windows**: Uses named pipes with WSADuplicateSocket
//!
//! The test harness binds the socket first, so connections queue in the TCP backlog
//! until the server is ready to accept.
//!
//! # Environment Variables
//!
//! - `DODECA_BIN`: Path to the ddc binary (required)
//! - `DODECA_TEST_WRAPPER`: Optional wrapper script/command to run ddc under
//!   (e.g., "valgrind --leak-check=full" or "strace -f -o /tmp/trace.out")
//! - `DODECA_HARNESS_HTTP_TIMEOUT_SECS`: Per-request HTTP timeout in seconds.
//! - `DODECA_HARNESS_RAW_TCP`: Set to "1" to enable raw TCP probe mode for
//!   connection diagnostics (measures connect/write/read phases separately)

use facet_value::Value;
use fs_err as fs;
use globset::Glob;
use hotmeal::{Document, NodeId, NodeKind, StrTendril};
use owo_colors::OwoColorize;
use regex::Regex;
use std::cell::Cell;
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::io::IntoRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::{debug, error, info};

const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 10;
static HTTP_TIMEOUT: OnceLock<Duration> = OnceLock::new();

fn http_timeout() -> Duration {
    *HTTP_TIMEOUT.get_or_init(|| {
        std::env::var("DODECA_HARNESS_HTTP_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS))
    })
}

// Thread-local storage for the active test id (used to route logs).
thread_local! {
    static CURRENT_TEST_ID: Cell<u64> = const { Cell::new(0) };
}

/// Per-test state storage
struct TestState {
    logs: Vec<LogLine>,
    exit_status: Option<std::process::ExitStatus>,
    start_time: Instant,
    setup_duration: Option<Duration>,
}

impl TestState {
    fn new() -> Self {
        Self {
            logs: Vec::new(),
            exit_status: None,
            start_time: Instant::now(),
            setup_duration: None,
        }
    }
}

/// Global test state storage (consolidated from 4 separate statics)
static TEST_STATES: OnceLock<Mutex<std::collections::HashMap<u64, TestState>>> = OnceLock::new();

/// Abbreviates long target paths to make logs more readable
fn abbreviate_target(target: &str) -> String {
    // Handle integration_tests targets
    if let Some(suffix) = target.strip_prefix("integration_tests::") {
        // Remove "integration_tests::"
        return format!("i_t::{}", suffix);
    }

    target.to_string()
}

#[derive(Clone, Debug, Copy, PartialEq, PartialOrd)]
enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl LogLevel {
    fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "ERROR" => LogLevel::Error,
            "WARN" => LogLevel::Warn,
            "INFO" => LogLevel::Info,
            "DEBUG" => LogLevel::Debug,
            "TRACE" => LogLevel::Trace,
            _ => LogLevel::Info, // default fallback
        }
    }

    fn format_colored(
        &self,
        target: &str,
        message: &str,
        fields: &std::collections::HashMap<String, String>,
    ) -> String {
        let abbreviated_target = abbreviate_target(target);
        let colored_target = abbreviated_target.truecolor(169, 177, 214);

        // Format structured fields with distinct colors
        let mut formatted_fields = String::new();
        if !fields.is_empty() {
            let field_parts: Vec<String> = fields
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}={}",
                        key.truecolor(158, 206, 106), // Green for keys
                        value.truecolor(224, 175, 104)
                    ) // Orange for values
                })
                .collect();
            formatted_fields = format!(" {}", field_parts.join(" "));
        }

        let base_message = match self {
            LogLevel::Error => format!(
                "{} {}: {}",
                "ERROR".truecolor(247, 118, 142).bold(),
                colored_target,
                message
            ),
            LogLevel::Warn => format!(
                "{:5} {}: {}",
                "WARN".truecolor(255, 158, 100).bold(),
                colored_target,
                message
            ),
            LogLevel::Info => format!(
                "{:5} {}: {}",
                "INFO".truecolor(122, 162, 247).bold(),
                colored_target,
                message
            ),
            LogLevel::Debug => format!(
                "{} {}: {}",
                "DEBUG".truecolor(187, 154, 247).bold(),
                colored_target,
                message
            ),
            LogLevel::Trace => format!(
                "{} {}: {}",
                "TRACE".truecolor(86, 95, 137).bold(),
                colored_target,
                message
            ),
        };

        format!("{}{}", base_message, formatted_fields)
    }
}

#[derive(Clone)]
struct LogLine {
    ts: Duration,
    abs: SystemTime,
    level: LogLevel,
    target: String,
    line: String,
    fields: std::collections::HashMap<String, String>,
}

// push_log function removed - all logging now goes through tracing

/// Set the current test id for log routing and initialize test state
pub fn set_current_test_id(id: u64) {
    CURRENT_TEST_ID.with(|cell| cell.set(id));

    // Initialize test state
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    states.lock().unwrap().insert(id, TestState::new());
}

/// Push a log entry for a specific test (used by tracing integration)
pub fn push_test_log(test_id: u64, level: &str, target: &str, message: &str) {
    push_test_log_with_fields(
        test_id,
        level,
        target,
        message,
        std::collections::HashMap::new(),
    );
}

/// Push a log entry with structured fields for a specific test
pub fn push_test_log_with_fields(
    test_id: u64,
    level: &str,
    target: &str,
    message: &str,
    fields: std::collections::HashMap<String, String>,
) {
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut states_map = states.lock().unwrap();

    if let Some(state) = states_map.get_mut(&test_id) {
        let log_entry = LogLine {
            ts: state.start_time.elapsed(),
            abs: SystemTime::now(),
            level: LogLevel::from_str(level),
            target: target.to_string(),
            line: message.to_string(),
            fields,
        };
        state.logs.push(log_entry);
    }
}

/// Clear per-test state
pub fn clear_test_state(id: u64) {
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    states.lock().unwrap().remove(&id);
}

/// Get the logs for a test id (for printing on failure)
pub fn get_logs_for(id: u64) -> Vec<String> {
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let states_map = states.lock().unwrap();
    if let Some(state) = states_map.get(&id) {
        render_logs(state.logs.clone())
    } else {
        Vec::new()
    }
}

/// Get the exit status for a test id (if the server already exited)
pub fn get_exit_status_for(id: u64) -> Option<std::process::ExitStatus> {
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let states_map = states.lock().unwrap();
    states_map.get(&id).and_then(|state| state.exit_status)
}

/// Get the setup duration for a test id
pub fn get_setup_for(id: u64) -> Option<Duration> {
    let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let states_map = states.lock().unwrap();
    states_map.get(&id).and_then(|state| state.setup_duration)
}

/// Get the path to the ddc binary.
///
/// `DODECA_BIN` remains an override for CI and wrappers. For local runs, infer
/// the sibling `ddc` binary next to the integration test runner.
pub fn ddc_binary() -> PathBuf {
    if let Ok(path) = std::env::var("DODECA_BIN") {
        return PathBuf::from(path);
    }

    let exe = std::env::current_exe().expect("get current executable path");
    let Some(dir) = exe.parent() else {
        panic!(
            "current executable has no parent directory: {}",
            exe.display()
        );
    };
    let inferred = dir.join(format!("ddc{}", std::env::consts::EXE_SUFFIX));
    if inferred.exists() {
        return inferred;
    }

    panic!(
        "DODECA_BIN is not set and inferred ddc binary does not exist at {}",
        inferred.display()
    );
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
    pub port: u16,
    fixture_dir: PathBuf,
    client: reqwest::Client,
    _temp_dir: tempfile::TempDir,
    #[cfg(unix)]
    _unix_socket_dir: tempfile::TempDir,
    test_id: u64,
}

/// Initialize the copied fixture as a git repo tracking a bare `origin` remote.
///
/// Lays out, all inside the fixture temp dir (so it's cleaned up with the site):
/// - `<fixture>/.git`     — the served working checkout
/// - `<fixture>/.origin.git` — a bare repo registered as `origin` (gitignored)
///
/// Commits the fixture content to `main`, then `git push -u origin main`. The
/// editor's git operations (`commit_as_user` → `fetch_rebase_push`) run against
/// this `origin`; a test can clone `.origin.git` again to push a concurrent
/// commit and exercise the non-fast-forward path.
fn init_git_with_origin(fixture_dir: &Path) {
    let bare = fixture_dir.join(".origin.git");
    run_git(fixture_dir, &["init", "-b", "main"]);
    run_git(fixture_dir, &["add", "-A"]);
    run_git(
        fixture_dir,
        &[
            "-c",
            "user.email=fixture@localhost",
            "-c",
            "user.name=fixture",
            "commit",
            "-m",
            "initial fixture content",
        ],
    );
    run_git(
        bare.parent().unwrap(),
        &["init", "--bare", bare.to_str().expect("utf8 origin path")],
    );
    run_git(
        fixture_dir,
        &[
            "remote",
            "add",
            "origin",
            bare.to_str().expect("utf8 origin path"),
        ],
    );
    run_git(fixture_dir, &["push", "-u", "origin", "main"]);
}

/// Run `git -C <cwd> <args>`, panicking with stderr on failure. Used only by the
/// harness's fixture-repo setup (the dev-editor save tests).
pub fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?} in {}: {e}", cwd.display()));
    if !output.status.success() {
        panic!(
            "git {args:?} in {} failed:\n{}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn fixture_root_dir() -> PathBuf {
    if let Ok(base) = std::env::var("DODECA_TEST_FIXTURES_DIR") {
        return PathBuf::from(base);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn fixture_source_dir(fixture_name: &str) -> PathBuf {
    let root = fixture_root_dir();
    let dir = root.join(fixture_name);
    if !dir.is_dir() {
        panic!(
            "fixture directory '{fixture_name}' not found at {} (fixtures root: {}).\n\
Hint: set DODECA_TEST_FIXTURES_DIR to the fixtures root and rebuild the integration harness if needed.",
            dir.display(),
            root.display()
        );
    }
    dir
}

impl TestSite {
    /// Create a new test site from a fixture directory name
    pub fn new(fixture_name: &str) -> Self {
        let src = fixture_source_dir(fixture_name);
        Self::from_source(&src)
    }

    /// Create a new test site from a fixture with custom files written before server start.
    /// This is useful for tests that need custom templates or content that should be
    /// loaded at server startup time rather than triggering livereload.
    pub fn with_files(fixture_name: &str, files: &[(&str, &str)]) -> Self {
        let src = fixture_source_dir(fixture_name);
        Self::from_source_with_files(&src, files)
    }

    /// Create a new test site from a fixture with custom setup before server start.
    pub fn with_setup<F>(fixture_name: &str, setup: F) -> Self
    where
        F: FnOnce(&Path),
    {
        let src = fixture_source_dir(fixture_name);
        Self::from_source_with_setup(&src, &[], setup)
    }

    /// Create a new test site from a fixture, served as a git repo behind the
    /// in-browser editor.
    ///
    /// Turns the copied fixture source into a git repository whose `main` branch
    /// tracks a **bare `origin`** remote, then spawns `ddc serve --dev-editor
    /// <user>` so the editor RPC works without an oauth2-proxy in front (the
    /// `<user>` becomes the editing identity, e.g. its commits are authored as
    /// `<user> <user@localhost>`). The bare origin lets a test simulate a
    /// concurrent push (advancing `origin/main`) between an editor load and save
    /// — the scenario the "fetch before push" fix in `commit_as_user` handles.
    pub fn with_editor_repo(fixture_name: &str, dev_user: &str) -> Self {
        let src = fixture_source_dir(fixture_name);
        Self::from_source_with_setup_and_editor(
            &src,
            &[],
            init_git_with_origin,
            Some(dev_user.to_string()),
        )
    }

    /// Create a new test site from an arbitrary source directory
    pub fn from_source(src: &Path) -> Self {
        Self::from_source_with_files(src, &[])
    }

    /// Create a new test site from an arbitrary source directory with custom files
    pub fn from_source_with_files(src: &Path, files: &[(&str, &str)]) -> Self {
        Self::from_source_with_setup(src, files, |_| {})
    }

    fn from_source_with_setup<F>(src: &Path, files: &[(&str, &str)], setup: F) -> Self
    where
        F: FnOnce(&Path),
    {
        Self::from_source_with_setup_and_editor(src, files, setup, None)
    }

    fn from_source_with_setup_and_editor<F>(
        src: &Path,
        files: &[(&str, &str)],
        setup: F,
        dev_editor: Option<String>,
    ) -> Self
    where
        F: FnOnce(&Path),
    {
        let setup_start = Instant::now();
        let test_id = CURRENT_TEST_ID.with(|cell| cell.get());
        // Create isolated temp directory
        let temp_dir = tempfile::Builder::new()
            .prefix("dodeca-test-")
            .tempdir()
            .expect("create temp dir");

        let fixture_dir = temp_dir.path().to_path_buf();
        copy_dir_recursive(src, &fixture_dir).unwrap_or_else(|e| {
            panic!(
                "copy fixture {} -> {}: {e}",
                src.display(),
                fixture_dir.display()
            )
        });

        // Write any custom files before starting the server
        for (rel_path, content) in files {
            let path = fixture_dir.join(rel_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dir for custom file");
            }
            fs::write(&path, content)
                .unwrap_or_else(|e| panic!("write custom file {}: {e}", path.display()));
        }

        setup(&fixture_dir);

        // Ensure .cache exists and is empty
        let cache_dir = fixture_dir.join(".cache");
        let _ = fs::remove_dir_all(&cache_dir);
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // Drive async socket operations on the ambient multi-threaded runtime. The
        // constructor stays synchronous: `block_in_place` lets us `block_on` the
        // fd-pass handshake without spinning up a nested runtime (which would panic
        // when called from within the runner's runtime).
        let rt = tokio::runtime::Handle::current();

        // Bind TCP socket on port 0 (OS assigns port) using std (not tokio)
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind TCP");
        let port = std_listener.local_addr().expect("get local addr").port();
        debug!(port, "Bound ephemeral TCP listener for test server");

        // Platform-specific: create control channel and get socket path for ddc
        #[cfg(unix)]
        let (unix_socket_dir, control_channel_path) = {
            let unix_socket_dir = tempfile::Builder::new()
                .prefix("dodeca-sock-")
                .tempdir()
                .expect("create unix socket dir");
            let unix_socket_path = unix_socket_dir.path().join("server.sock");
            let path_str = unix_socket_path.to_string_lossy().to_string();
            (unix_socket_dir, path_str)
        };

        #[cfg(windows)]
        let control_channel_path = {
            // Use a unique named pipe for this test
            let pipe_name = format!(
                r"\\.\pipe\dodeca-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            pipe_name
        };

        // Start server with control channel path
        let fixture_str = fixture_dir.to_string_lossy().to_string();
        let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| {
            // Default to debug for everything, but turn off specific noisy modules
            "debug,ureq=warn,hyper=warn,h2=warn,tower=warn,tonic=warn".to_string()
        });

        let ddc = ddc_binary();
        let mut ddc_args: Vec<&str> = vec![
            "serve",
            &fixture_str,
            "--no-tui",
            "--fd-socket",
            &control_channel_path,
        ];
        // `--dev-editor <user>` bypasses oauth2-proxy so the in-browser editor RPC
        // works without forwarded auth headers; `<user>` becomes the editing
        // identity. Refused on a non-loopback bind, so it's local-test only.
        if let Some(user) = dev_editor.as_deref() {
            ddc_args.push("--dev-editor");
            ddc_args.push(user);
        }

        // Build command, optionally wrapping with DODECA_TEST_WRAPPER
        let mut cmd = if let Some(wrapper_parts) = test_wrapper() {
            let (wrapper_cmd, wrapper_args) = wrapper_parts
                .split_first()
                .expect("DODECA_TEST_WRAPPER must not be empty");
            let mut cmd = StdCommand::new(wrapper_cmd);
            cmd.args(wrapper_args);
            cmd.arg(&ddc);
            cmd.args(&ddc_args);
            debug!(wrapper = %wrapper_cmd, "Using test wrapper");
            cmd
        } else {
            let mut cmd = StdCommand::new(&ddc);
            cmd.args(&ddc_args);
            cmd
        };

        cmd.env("RUST_LOG", &rust_log)
            .env("RUST_BACKTRACE", "1")
            .env("DDC_LOG_TIME", "none")
            .env("DODECA_QUIET", "1")
            .env("DDC_LOG_FORMAT", "json")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Keep code-exec builds fast and isolated per test (avoids cross-test contention under
        // nextest parallelism).
        let code_exec_target_dir = temp_dir.path().join("code-exec-target");
        let _ = fs::create_dir_all(&code_exec_target_dir);
        cmd.env("DDC_CODE_EXEC_TARGET_DIR", &code_exec_target_dir);

        // Enable death-watch so ddc (and its cells) die when the test process dies.
        // This prevents orphan accumulation when tests are killed or crash.
        cmd.env("DODECA_DIE_WITH_PARENT", "1");

        // Editor mode: deny ddc any ambient git identity, reproducing the
        // deployed pod (which has none — commits are made with per-call `-c`).
        // This makes the editor-save test a real oracle for the rebase
        // committer-identity path in fetch_rebase_push; without it the test
        // would pass spuriously on a dev machine that has a global git user.
        if dev_editor.is_some() {
            cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null");
        }

        // Platform-specific: set up control channel listener before spawning
        #[cfg(unix)]
        let unix_listener = {
            let unix_socket_path = std::path::Path::new(&control_channel_path);
            tokio::task::block_in_place(|| {
                rt.block_on(async {
                    UnixListener::bind(unix_socket_path).expect("bind Unix socket")
                })
            })
        };

        #[cfg(windows)]
        let mut pipe_listener = tokio::task::block_in_place(|| {
            rt.block_on(async {
                vox_stream::LocalListener::bind(&control_channel_path).expect("bind named pipe")
            })
        });

        let mut child = ur_taking_me_with_you::spawn_dying_with_parent(cmd)
            .expect("start server with death-watch");
        debug!(child_pid = child.id(), ddc = %ddc.display(), %rust_log, "Spawned ddc server process");

        // Take stdout/stderr before the async block
        let stdout = child.stdout.take().expect("capture stdout");
        let stderr = child.stderr.take().expect("capture stderr");

        // Prepare readers for background log capture
        let stdout_reader = BufReader::new(stdout);
        let stderr_reader = BufReader::new(stderr);
        info!("spawned ddc pid={}", child.id());

        // Start background threads to capture logs BEFORE fd-socket handling
        // This ensures we capture any early output from ddc
        std::thread::spawn(move || {
            for line in stdout_reader.lines() {
                match line {
                    Ok(l) => {
                        process_ddc_log_line(&l, "stdout", test_id);
                    }
                    Err(_) => break,
                }
            }
        });

        std::thread::spawn(move || {
            for line in stderr_reader.lines() {
                match line {
                    Ok(l) => {
                        process_ddc_log_line(&l, "stderr", test_id);
                    }
                    Err(_) => break,
                }
            }
        });

        debug!("log capture threads started");

        // Platform-specific: accept connection and send TCP listener
        let child_id = child.id();

        #[cfg(unix)]
        {
            let listener_fd = std_listener.into_raw_fd();
            tokio::task::block_in_place(|| {
                rt.block_on(async {
                use tokio::io::AsyncReadExt;

                debug!("waiting for unix socket accept");
                let accept_future = unix_listener.accept();
                let timeout_duration = tokio::time::Duration::from_secs(5);

                let (mut unix_stream, _) = tokio::time::timeout(timeout_duration, accept_future)
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Timeout waiting for server (PID {}) to connect to Unix socket within 5s",
                            child_id
                        )
                    })
                    .expect("Failed to accept Unix connection");

                // Send the TCP listener FD to the server
                vox_fdpass::send_fd(&unix_stream, listener_fd)
                    .await
                    .expect("send FD");
                info!("sent TCP listener FD to ddc");
                debug!("Sent TCP listener FD to server");

                // FD passing ack:
                //
                // The server must confirm it has received and adopted the listening FD before the
                // harness closes its copy. This eliminates OS-specific edge cases (notably on macOS)
                // where closing too early can lead to transient ECONNRESET/ECONNREFUSED for the very
                // first test request.
                let mut ack_buf = [0u8; 1];
                tokio::time::timeout(timeout_duration, unix_stream.read_exact(&mut ack_buf))
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Timeout waiting for server (PID {}) to ack FD receipt within 5s",
                            child_id
                        )
                    })
                    .expect("Failed to read FD ack from server");
                if ack_buf != [0xAC] {
                    panic!(
                        "Unexpected FD ack byte from server (PID {}): got 0x{:02x}, expected 0xAC",
                        child_id, ack_buf[0]
                    );
                }
                info!("server acked listener FD");

                // Close the local copy *after* the server has acked receipt. The receiver gets its
                // own duplicate FD via SCM_RIGHTS.
                use std::os::fd::FromRawFd;

                // SAFETY: We created `listener_fd` from a freshly-bound listener and haven't closed it.
                let _ = unsafe { std::os::fd::OwnedFd::from_raw_fd(listener_fd) };
            })
            });
        }

        #[cfg(windows)]
        {
            tokio::task::block_in_place(|| {
                rt.block_on(async {
                use tokio::io::AsyncReadExt;

                debug!("waiting for named pipe accept");
                let timeout_duration = tokio::time::Duration::from_secs(5);

                let mut pipe_stream = tokio::time::timeout(timeout_duration, pipe_listener.accept())
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Timeout waiting for server (PID {}) to connect to named pipe within 5s",
                            child_id
                        )
                    })
                    .expect("Failed to accept named pipe connection");

                // Send the TCP listener to the server using WSADuplicateSocket
                vox_fdpass::send_tcp_listener(&mut pipe_stream, &std_listener, child_id)
                    .await
                    .expect("send TCP listener");
                info!("sent TCP listener to ddc");
                debug!("Sent TCP listener to server via WSADuplicateSocket");

                // Socket passing ack
                let mut ack_buf = [0u8; 1];
                tokio::time::timeout(timeout_duration, pipe_stream.read_exact(&mut ack_buf))
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "Timeout waiting for server (PID {}) to ack socket receipt within 5s",
                            child_id
                        )
                    })
                    .expect("Failed to read socket ack from server");
                if ack_buf != [0xAC] {
                    panic!(
                        "Unexpected socket ack byte from server (PID {}): got 0x{:02x}, expected 0xAC",
                        child_id, ack_buf[0]
                    );
                }
                info!("server acked listener socket");
            })
            });
        }

        let setup_elapsed = setup_start.elapsed();
        let states = TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
        if let Some(state) = states.lock().unwrap().get_mut(&test_id) {
            state.setup_duration = Some(setup_elapsed);
        }

        let site = Self {
            child,
            port,
            fixture_dir,
            client: harness_http_client(),
            _temp_dir: temp_dir,
            #[cfg(unix)]
            _unix_socket_dir: unix_socket_dir,
            test_id,
        };

        site
    }

    /// Clear captured logs for this test
    pub fn clear_logs(&self) {
        clear_test_state(self.test_id);
    }

    /// Return the current log cursor (number of log lines)
    pub fn log_cursor(&self) -> usize {
        let logs = get_logs_for(self.test_id);
        logs.len()
    }

    /// Count log lines containing `needle` since `cursor`
    pub fn count_logs_since(&self, cursor: usize, needle: &str) -> usize {
        let logs = get_logs_for(self.test_id);
        logs.iter()
            .skip(cursor)
            .filter(|line| line.contains(needle))
            .count()
    }

    /// Make a GET request to a path
    ///
    /// No retries - a failure fails the test immediately. The server contract
    /// guarantees connections are never refused/reset during boot; they may
    /// stall until ready, but connect+write must never fail.
    pub async fn get(&self, path: &str) -> Response {
        self.get_with_timeout(path, http_timeout()).await
    }

    async fn get_with_timeout(&self, path: &str, timeout: Duration) -> Response {
        let url = format!("http://127.0.0.1:{}{}", self.port, path);
        debug!("→ GET {}", path);

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

        let request_start = Instant::now();
        match self.client.get(&url).timeout(timeout).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();

                // Extract interesting headers (clone strings before consuming resp)
                let headers = resp.headers();
                let content_type = headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("(none)")
                    .to_string();
                let generation = headers
                    .get("x-picante-generation")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("?")
                    .to_string();

                let body = resp.text().await.unwrap_or_default();
                let body_len = body.len();
                let elapsed_ms = request_start.elapsed().as_millis();

                debug!(
                    "← {} {} ({} ms, {} bytes, gen={}, content-type: {})",
                    status, path, elapsed_ms, body_len, generation, content_type
                );

                Response {
                    status,
                    body,
                    url,
                    content_type,
                }
            }
            Err(e) => {
                let elapsed_ms = request_start.elapsed().as_millis();
                error!("✗ GET {} failed after {} ms: {}", path, elapsed_ms, e);
                error!(%url, error = ?e, "GET failed (no retries)");
                panic!(
                    "GET {} failed after {:?}:\n{:?}\n{}",
                    url,
                    timeout,
                    e,
                    format_error_chain(&e)
                );
            }
        }
    }

    /// GET a path and return the raw response body bytes. Panics on a non-200
    /// status or transport error. Use this for binary assets — the search
    /// index files are postcard, which a lossy `String` would corrupt.
    pub async fn get_bytes(&self, path: &str) -> Vec<u8> {
        let url = format!("http://127.0.0.1:{}{}", self.port, path);
        debug!("→ GET (bytes) {}", path);
        match self.client.get(&url).timeout(http_timeout()).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                assert_eq!(status, 200, "GET {path}: expected 200, got {status}");
                resp.bytes()
                    .await
                    .unwrap_or_else(|e| panic!("GET {path}: read body: {e}"))
                    .to_vec()
            }
            Err(e) => panic!("GET {path} failed: {e:?}"),
        }
    }

    /// Raw TCP probe for instrumentation. Measures connect, write, and read phases
    /// separately to diagnose where failures occur.
    ///
    /// Enable with `DODECA_HARNESS_RAW_TCP=1` env var.
    ///
    /// Returns timing info as a log message; does not parse the response.
    #[allow(dead_code)]
    pub async fn probe_tcp(&self, path: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let addr = format!("127.0.0.1:{}", self.port);
        info!(target: "probe", "Starting raw TCP probe to {}{}", addr, path);

        // Phase 1: Connect
        let connect_start = Instant::now();
        let mut stream = match TcpStream::connect(&addr).await {
            Ok(s) => s,
            Err(e) => {
                let elapsed = connect_start.elapsed();
                let msg = format!(
                    "[probe] CONNECT FAILED after {:?}: {} (kind={:?})",
                    elapsed,
                    e,
                    e.kind()
                );
                error!(target: "probe", "{}", msg);
                panic!("{}", msg);
            }
        };
        let connect_elapsed = connect_start.elapsed();
        info!(target: "probe", "Connected in {:?}", connect_elapsed);

        // Phase 2: Write HTTP request
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            path, self.port
        );
        let write_start = Instant::now();
        if let Err(e) = stream.write_all(request.as_bytes()).await {
            let elapsed = write_start.elapsed();
            let msg = format!(
                "[probe] WRITE FAILED after {:?}: {} (kind={:?})",
                elapsed,
                e,
                e.kind()
            );
            error!(target: "probe", "{}", msg);
            panic!("{}", msg);
        }
        let write_elapsed = write_start.elapsed();
        info!(target: "probe", "Wrote request in {:?} ({} bytes)", write_elapsed, request.len());

        // Phase 3: Read first byte (with timeout)
        let read_start = Instant::now();
        let mut buf = [0u8; 1];
        match tokio::time::timeout(Duration::from_secs(30), stream.read(&mut buf)).await {
            Ok(Ok(0)) => {
                let elapsed = read_start.elapsed();
                let msg = format!(
                    "[probe] READ EOF after {:?} (server closed without response)",
                    elapsed
                );
                debug!(target: "probe", "{}", msg);
            }
            Ok(Ok(_)) => {
                let elapsed = read_start.elapsed();
                let msg = format!(
                    "[probe] READ OK in {:?} (first byte: 0x{:02x} '{}')",
                    elapsed,
                    buf[0],
                    if buf[0].is_ascii_graphic() || buf[0] == b' ' {
                        buf[0] as char
                    } else {
                        '.'
                    }
                );
                debug!(target: "probe", "{}", msg);
            }
            Ok(Err(e)) => {
                let elapsed = read_start.elapsed();
                let msg = format!(
                    "[probe] READ FAILED after {:?}: {} (kind={:?})",
                    elapsed,
                    e,
                    e.kind()
                );
                // Read failures during probe are logged but not fatal
                debug!(target: "probe", "{}", msg);
            }
            Err(_elapsed) => {
                let elapsed = read_start.elapsed();
                let msg = format!("[probe] READ TIMED OUT after {:?}", elapsed);
                // Read timeouts during probe are logged but not fatal
                debug!(target: "probe", "{}", msg);
            }
        }

        let total = connect_start.elapsed();
        info!(target: "probe", "COMPLETE: connect={:?} write={:?} total={:?}",
              connect_elapsed, write_elapsed, total);
    }

    /// Check if raw TCP probe mode is enabled via DODECA_HARNESS_RAW_TCP=1
    #[allow(dead_code)]
    pub fn raw_tcp_enabled() -> bool {
        std::env::var("DODECA_HARNESS_RAW_TCP")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// Wait until a condition is true, retrying until timeout
    /// Returns the value produced by the condition, or panics on timeout
    pub async fn wait_until<T, F>(&self, desc: &str, timeout: Duration, mut condition: F) -> T
    where
        F: AsyncFnMut() -> Option<T>,
    {
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(value) = condition().await {
                return value;
            }

            if Instant::now() >= deadline {
                break;
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        panic!("Condition '{}' not met within {:?}", desc, timeout);
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
    pub async fn wait_debounce(&self) {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

impl Drop for TestSite {
    fn drop(&mut self) {
        info!("cleaning up ddc pid={}", self.child.id());

        match self.child.try_wait() {
            Ok(Some(status)) => {
                info!("ddc already exited status={status}");
                let states =
                    TEST_STATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
                if let Some(state) = states.lock().unwrap().get_mut(&self.test_id) {
                    state.exit_status = Some(status);
                }
            }
            Ok(None) => {
                info!("killing ddc");
                if let Err(e) = self.child.kill() {
                    error!("kill failed: {e}");
                }

                match self.child.wait() {
                    Ok(status) => {
                        info!("wait complete status={status}");
                        let states = TEST_STATES
                            .get_or_init(|| Mutex::new(std::collections::HashMap::new()));
                        if let Some(state) = states.lock().unwrap().get_mut(&self.test_id) {
                            state.exit_status = Some(status);
                        }
                    }
                    Err(e) => {
                        error!("wait failed: {e}");
                    }
                }
            }
            Err(e) => {
                error!("try_wait failed: {e}");
            }
        }

        // Logs are now handled by the tracing system via push_test_log
        // No need to copy from self.logs since it's no longer used
    }
}

fn harness_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .build()
        .expect("build reqwest client")
}

fn render_logs(mut lines: Vec<LogLine>) -> Vec<String> {
    lines.sort_by_key(|l| {
        l.abs
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
    });

    // Parse RUST_LOG to determine filtering
    let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let should_show_log = |level: LogLevel, target: &str| -> bool {
        if rust_log.is_empty() {
            return true; // Show all if no RUST_LOG set
        }

        // Simple parsing - look for patterns like "debug", "info", "target=level"
        let rust_log_lower = rust_log.to_lowercase();

        // Check for target-specific rules first (e.g., "ddc=info")
        for directive in rust_log_lower.split(',') {
            let directive = directive.trim();
            if let Some((target_pattern, level_str)) = directive.split_once('=')
                && target.starts_with(target_pattern.trim())
            {
                let directive_level = LogLevel::from_str(level_str.trim());
                return level <= directive_level;
            }
        }

        // Check for global level (e.g., just "debug" in RUST_LOG)
        for directive in rust_log_lower.split(',') {
            let directive = directive.trim();
            if !directive.contains('=') {
                // This is a global level directive
                let directive_level = LogLevel::from_str(directive);
                return level <= directive_level;
            }
        }

        // Default to info level if no matching directive found
        level <= LogLevel::Info
    };

    lines
        .into_iter()
        .filter(|l| should_show_log(l.level, &l.target))
        .map(|l| {
            let timestamp = format!("{:>5.3}s", l.ts.as_secs_f64())
                .truecolor(65, 72, 104)
                .to_string();

            // Use structured data for consistent coloring
            let colored_message = l.level.format_colored(&l.target, &l.line, &l.fields);
            format!("{} {}", timestamp, colored_message)
        })
        .collect()
}

fn matches_glob(pattern: &str, value: &str) -> bool {
    match Glob::new(pattern) {
        Ok(glob) => glob.compile_matcher().is_match(value),
        Err(err) => {
            tracing::debug!("Invalid glob pattern '{}': {}", pattern, err);
            false
        }
    }
}

fn find_attr_in_node<F>(
    doc: &Document,
    node_id: NodeId,
    tag: &str,
    attr: &str,
    matcher: &F,
) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let node = doc.get(node_id);
    if let NodeKind::Element(elem) = &node.kind
        && elem.tag.as_ref() == tag
    {
        for (name, value) in &elem.attrs {
            if name.local.as_ref() == attr {
                let value_str = value.as_ref();
                if matcher(value_str) {
                    return Some(value_str.to_string());
                }
            }
        }
    }

    for child_id in doc.children(node_id) {
        if let Some(found) = find_attr_in_node(doc, child_id, tag, attr, matcher) {
            return Some(found);
        }
    }

    None
}

/// An HTTP response
pub struct Response {
    pub status: u16,
    pub body: String,
    pub url: String,
    pub content_type: String,
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

    /// Assert the response content type starts with a prefix.
    pub fn assert_content_type(&self, expected: &str) {
        assert!(
            self.content_type.starts_with(expected),
            "Expected content-type for {} to start with '{}', got '{}'",
            self.url,
            expected,
            self.content_type
        );
    }

    /// Get the body text
    pub fn text(&self) -> &str {
        &self.body
    }

    /// Find an <img> tag's src attribute matching a glob pattern
    /// Returns the matched src value (without host) or None
    pub fn img_src(&self, pattern: &str) -> Option<String> {
        let tendril = StrTendril::from(self.body.as_str());
        let doc = hotmeal::parse(&tendril);
        find_attr_in_node(&doc, doc.root, "img", "src", &|value| {
            matches_glob(pattern, value)
        })
    }

    /// Find a <link> tag's href attribute matching a glob pattern
    /// Returns the matched href value (without host) or None
    pub fn css_link(&self, pattern: &str) -> Option<String> {
        let tendril = StrTendril::from(self.body.as_str());
        let doc = hotmeal::parse(&tendril);
        find_attr_in_node(&doc, doc.root, "link", "href", &|value| {
            matches_glob(pattern, value)
        })
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
    let entries = fs::read_dir(src)?;

    for entry in entries {
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

        // Write config (Styx format)
        fs::write(
            fixture_dir.join(".config/dodeca.styx"),
            "content content\noutput public\n",
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

    pub fn build_in_place(&self) -> BuildResult {
        fs::create_dir_all(self.fixture_dir.join("public")).expect("create output dir");

        let fixture_str = self.fixture_dir.to_string_lossy().to_string();
        let ddc = ddc_binary();
        let mut cmd = StdCommand::new(&ddc);
        cmd.args(["build", &fixture_str]);

        let code_exec_target_dir = self.fixture_dir.join(".cache/code-exec-target");
        let _ = fs::create_dir_all(&code_exec_target_dir);
        cmd.env("DDC_CODE_EXEC_TARGET_DIR", &code_exec_target_dir);

        let output = cmd.output().expect("run build");

        BuildResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
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
    copy_dir_recursive(src, &fixture_dir).unwrap_or_else(|e| {
        panic!(
            "copy fixture {} -> {}: {e}",
            src.display(),
            fixture_dir.display()
        )
    });

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

/// Process a log line from ddc, attempting JSON parsing first, then falling back to text
fn process_ddc_log_line(line: &str, stream: &str, test_id: u64) {
    let trimmed = line.trim();

    // Try to parse as JSON first
    if let Ok(json_value) = facet_json::from_str::<Value>(trimmed)
        && let Some(parsed) = parse_json_log(&json_value)
    {
        // Successfully parsed JSON log - push structured data to test log
        push_test_log_with_fields(
            test_id,
            &parsed.level,
            &parsed.target,
            &parsed.message,
            parsed.fields,
        );
        return;
    }

    // Fallback to text parsing
    if trimmed.starts_with("INFO ")
        || trimmed.starts_with("DEBUG ")
        || trimmed.starts_with("WARN ")
        || trimmed.starts_with("ERROR ")
        || trimmed.starts_with("TRACE ")
    {
        // Parse pre-formatted logs to extract level, target, and message
        if let Some(space_pos) = trimmed.find(' ') {
            let level = &trimmed[..space_pos];
            if let Some(colon_pos) = trimmed.find(": ") {
                let target = &trimmed[space_pos + 1..colon_pos];
                let message = &trimmed[colon_pos + 2..];
                push_test_log(test_id, level, target, message);
            } else {
                // No target separator found, treat as unknown target
                let message = &trimmed[space_pos + 1..];
                push_test_log(test_id, level, "unknown", message);
            }
        } else {
            // No space found, treat entire line as message with unknown level
            push_test_log(test_id, "INFO", "unknown", trimmed);
        }
    } else {
        // Regular output without log formatting - push directly to avoid harness prefix
        let target = match stream {
            "stdout" => "stdout",
            "stderr" => "stderr",
            _ => "unknown",
        };
        // Use DEBUG level for unstructured stderr (like cell ready signals)
        // and INFO for stdout (like user-facing messages)
        let level = match stream {
            "stderr" => "DEBUG",
            _ => "INFO",
        };
        push_test_log(test_id, level, target, line);
    }
}

/// Parsed JSON log entry
struct ParsedJsonLog {
    level: String,
    target: String,
    message: String,
    fields: std::collections::HashMap<String, String>,
}

/// Parse a JSON log entry from tracing-subscriber
fn parse_json_log(json: &Value) -> Option<ParsedJsonLog> {
    use facet_value::DestructuredRef;

    // Destructure the value to access object fields
    let obj = match json.destructure_ref() {
        DestructuredRef::Object(obj) => obj,
        _ => return None, // Not an object
    };

    // Extract level
    let level = obj.get("level")?.destructure_ref();
    let level = match level {
        DestructuredRef::String(s) => s.to_string().to_uppercase(),
        _ => return None,
    };

    // Extract target
    let target = obj.get("target")?.destructure_ref();
    let target = match target {
        DestructuredRef::String(s) => s.to_string(),
        _ => return None,
    };

    // Extract message and structured fields - tracing-subscriber JSON uses different field structures
    let mut fields = std::collections::HashMap::new();
    let message = if let Some(fields_value) = obj.get("fields") {
        // Check if fields is an object with message
        match fields_value.destructure_ref() {
            DestructuredRef::Object(fields_obj) => {
                let mut message = None;

                // Extract all fields, treating message specially
                for (key, value) in fields_obj.iter() {
                    let key_str = key.as_str();
                    match value.destructure_ref() {
                        DestructuredRef::String(s) => {
                            if key_str == "message" {
                                message = Some(s.to_string());
                            } else {
                                fields.insert(key_str.to_string(), s.to_string());
                            }
                        }
                        _ => {
                            let formatted_value = facet_value::format_value(value);
                            if key_str == "message" {
                                message = Some(formatted_value);
                            } else {
                                fields.insert(key_str.to_string(), formatted_value);
                            }
                        }
                    }
                }

                // Use message if found, otherwise use target
                message.unwrap_or_else(|| target.clone())
            }
            _ => format!("fields: {}", facet_value::format_value(fields_value)),
        }
    } else if let Some(msg_value) = obj.get("message") {
        // Direct message field
        match msg_value.destructure_ref() {
            DestructuredRef::String(s) => s.to_string(),
            _ => target.clone(),
        }
    } else if let Some(msg_value) = obj.get("msg") {
        // Alternative message field name
        match msg_value.destructure_ref() {
            DestructuredRef::String(s) => s.to_string(),
            _ => target.clone(),
        }
    } else {
        target.clone() // fallback to target if no message found
    };

    // Extract additional top-level fields that aren't level, target, message, or timestamp
    for (key, value) in obj.iter() {
        let key_str = key.as_str();
        if ![
            "level",
            "target",
            "message",
            "msg",
            "fields",
            "timestamp",
            "time",
        ]
        .contains(&key_str)
        {
            match value.destructure_ref() {
                DestructuredRef::String(s) => {
                    fields.insert(key_str.to_string(), s.to_string());
                }
                _ => {
                    fields.insert(key_str.to_string(), facet_value::format_value(value));
                }
            }
        }
    }

    Some(ParsedJsonLog {
        level,
        target,
        message,
        fields,
    })
}
