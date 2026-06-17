use super::*;
use camino::{Utf8Path, Utf8PathBuf};
use facet_value::{Value, value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tower_lsp::lsp_types::Url;

const LIST_PAGES_COMMAND: &str = "dodeca.listPages";

struct LspSite {
    _temp_dir: tempfile::TempDir,
    root_dir: Utf8PathBuf,
    content_dir: Utf8PathBuf,
}

impl LspSite {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let root_dir = Utf8PathBuf::from_path_buf(temp_dir.path().to_path_buf())
            .expect("temp dir path is utf8");
        let content_dir = root_dir.join("content");
        let templates_dir = root_dir.join("templates");
        let static_dir = root_dir.join("static");

        fs_err::create_dir_all(&content_dir).expect("create content dir");
        fs_err::create_dir_all(&templates_dir).expect("create templates dir");
        fs_err::create_dir_all(&static_dir).expect("create static dir");

        fs_err::write(
            content_dir.join("_index.md"),
            "+++\ntitle = \"Home\"\n+++\n\n# Home\n",
        )
        .expect("write root section");
        fs_err::create_dir_all(content_dir.join("guide")).expect("create guide dir");
        fs_err::write(
            content_dir.join("guide/intro.md"),
            "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n\nSee [Home](/).\n",
        )
        .expect("write intro page");
        fs_err::write(
            templates_dir.join("index.html"),
            "{{ section.content | safe }}",
        )
        .expect("write index template");
        fs_err::write(
            templates_dir.join("section.html"),
            "{{ section.content | safe }}",
        )
        .expect("write section template");
        fs_err::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
            .expect("write page template");

        Self {
            _temp_dir: temp_dir,
            root_dir,
            content_dir,
        }
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.root_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs_err::create_dir_all(parent).expect("create parent dir");
        }
        fs_err::write(path, content).expect("write site file");
    }

    fn uri(&self, relative: &str) -> String {
        file_uri(&self.root_dir.join(relative)).to_string()
    }

    fn client(&self) -> LspClient {
        LspClient::start(&self.root_dir, &self.content_dir)
    }
}

struct LspClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    rx: mpsc::Receiver<Value>,
    next_id: u64,
    stderr: Arc<Mutex<String>>,
    logs: Vec<String>,
}

impl LspClient {
    fn start(root_dir: &Utf8Path, content_dir: &Utf8Path) -> Self {
        let ddc = ddc_binary();
        let mut cmd = std::process::Command::new(&ddc);
        cmd.arg("lsp")
            .arg("--content")
            .arg(content_dir.as_str())
            .arg("--output")
            .arg(root_dir.join("public").as_str())
            .env("RUST_BACKTRACE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().expect("spawn ddc lsp");
        let stdin = Arc::new(Mutex::new(child.stdin.take().expect("capture lsp stdin")));
        let stdout = child.stdout.take().expect("capture lsp stdout");
        let stderr_pipe = child.stderr.take().expect("capture lsp stderr");
        let stderr = Arc::new(Mutex::new(String::new()));

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = read_lsp_message(&mut reader) {
                if tx.send(message).is_err() {
                    break;
                }
            }
        });

        let stderr_capture = Arc::clone(&stderr);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr_pipe);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => stderr_capture.lock().unwrap().push_str(&line),
                }
            }
        });

        let mut client = Self {
            child,
            stdin,
            rx,
            next_id: 1,
            stderr,
            logs: Vec::new(),
        };
        client.initialize(root_dir);
        client
    }

    fn initialize(&mut self, root_dir: &Utf8Path) {
        let root_uri = file_uri(root_dir).to_string();
        let result = self.request(
            "initialize",
            value!({
                "processId": null,
                "rootUri": (root_uri.clone()),
                "workspaceFolders": [{
                    "uri": (root_uri),
                    "name": "dodeca-lsp-integration",
                }],
                "capabilities": {
                    "workspace": {
                        "didChangeWatchedFiles": {
                            "dynamicRegistration": true,
                        },
                    },
                    "textDocument": {
                        "completion": {},
                        "codeAction": {},
                        "documentSymbol": {},
                    },
                },
            }),
        );
        assert_eq!(
            result
                .pointer("/serverInfo/name")
                .and_then(LspJsonValue::as_str)
                .unwrap_or(""),
            "dodeca-authoring"
        );
        self.notify("initialized", value!({}));
    }

    fn list_pages(&mut self) -> Value {
        self.execute_command(LIST_PAGES_COMMAND, value!([]))
    }

    fn execute_command(&mut self, command: &str, arguments: Value) -> Value {
        self.request(
            "workspace/executeCommand",
            value!({
                "command": (command),
                "arguments": (arguments),
            }),
        )
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(value!({
            "jsonrpc": "2.0",
            "id": (id),
            "method": (method),
            "params": (params),
        }));
        self.wait_for_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(value!({
            "jsonrpc": "2.0",
            "method": (method),
            "params": (params),
        }));
    }

    fn send(&self, message: Value) {
        let body = facet_json::to_vec(&message).expect("serialize lsp message");
        let mut stdin = self.stdin.lock().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write lsp header");
        stdin.write_all(&body).expect("write lsp body");
        stdin.flush().expect("flush lsp message");
    }

    fn wait_for_response(&mut self, id: u64) -> Value {
        loop {
            let message = self.next_message();
            if self.handle_server_request(&message) {
                continue;
            }
            if self.capture_log_message(&message) {
                continue;
            }
            if message.get("id").and_then(LspJsonValue::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    self.drain_pending_logs();
                    let stderr = self.stderr.lock().unwrap().clone();
                    panic!(
                        "LSP request {id} failed: {error:#?}\nserver logs: {:#?}\nstderr:\n{stderr}",
                        self.logs
                    );
                }
                return message.get("result").cloned().unwrap_or(Value::NULL);
            }
        }
    }

    fn wait_for_notification<F>(&mut self, method: &str, mut predicate: F) -> Value
    where
        F: FnMut(&Value) -> bool,
    {
        loop {
            let message = self.next_message();
            if self.handle_server_request(&message) {
                continue;
            }
            if self.capture_log_message(&message) {
                continue;
            }
            if message.get("method").and_then(LspJsonValue::as_str) == Some(method) {
                let params = message.get("params").cloned().unwrap_or(Value::NULL);
                if predicate(&params) {
                    return params;
                }
            }
        }
    }

    fn next_message(&mut self) -> Value {
        self.rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| {
                let stderr = self.stderr.lock().unwrap().clone();
                panic!("timed out waiting for LSP message; stderr:\n{stderr}");
            })
    }

    fn handle_server_request(&mut self, message: &Value) -> bool {
        let Some(id) = message.get("id").cloned() else {
            return false;
        };
        if message.get("method").is_none() {
            return false;
        }
        self.send(value!({
            "jsonrpc": "2.0",
            "id": (id),
            "result": null,
        }));
        true
    }

    fn drain_pending_logs(&mut self) {
        while let Ok(message) = self.rx.recv_timeout(Duration::from_millis(100)) {
            if self.handle_server_request(&message) {
                continue;
            }
            let _ = self.capture_log_message(&message);
        }
    }

    fn capture_log_message(&mut self, message: &Value) -> bool {
        if message.get("method").and_then(LspJsonValue::as_str) != Some("window/logMessage") {
            return false;
        }
        if let Some(text) = message
            .pointer("/params/message")
            .and_then(LspJsonValue::as_str)
            .map(str::to_string)
        {
            self.logs.push(text);
        }
        true
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let id = self.next_id;
            self.next_id += 1;
            self.send(value!({
                "jsonrpc": "2.0",
                "id": (id),
                "method": "shutdown",
                "params": null,
            }));
            let _ = self.wait_for_response(id);
            self.notify("exit", Value::NULL);
        }));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_lsp_message<R: Read>(reader: &mut BufReader<R>) -> Option<Value> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            return None;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().expect("content length"));
        }
    }

    let len = content_length.expect("lsp message content length");
    let mut body = vec![0; len];
    reader.read_exact(&mut body).ok()?;
    Some(facet_json::from_slice(&body).expect("parse lsp message"))
}

fn file_uri(path: &Utf8Path) -> Url {
    Url::from_file_path(path.as_std_path()).expect("file uri")
}

trait LspJsonValue {
    fn get(&self, key: &str) -> Option<&Value>;
    fn pointer(&self, pointer: &str) -> Option<&Value>;
    fn as_str(&self) -> Option<&str>;
    fn as_u64(&self) -> Option<u64>;
}

impl LspJsonValue for Value {
    fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    fn pointer(&self, pointer: &str) -> Option<&Value> {
        pointer
            .strip_prefix('/')
            .unwrap_or(pointer)
            .split('/')
            .filter(|segment| !segment.is_empty())
            .try_fold(self, |value, segment| value.get(segment))
    }

    fn as_str(&self) -> Option<&str> {
        self.as_string().map(|string| string.as_str())
    }

    fn as_u64(&self) -> Option<u64> {
        self.as_number()?.to_u64()
    }
}

fn page_routes(pages: &Value) -> Vec<String> {
    pages
        .as_array()
        .expect("pages array")
        .iter()
        .filter_map(|page| page.get("route").and_then(LspJsonValue::as_str))
        .map(str::to_string)
        .collect()
}

pub async fn lsp_lists_pages_over_stdio() {
    let site = LspSite::new();
    let mut client = site.client();

    let pages = client.list_pages();
    let routes = page_routes(&pages);

    assert!(routes.iter().any(|route| route == "/"));
    assert!(routes.iter().any(|route| route == "/guide/intro"));
}

pub async fn lsp_uses_open_document_overlays() {
    let site = LspSite::new();
    let mut client = site.client();
    let uri = site.uri("content/draft.md");

    client.notify(
        "textDocument/didOpen",
        value!({
            "textDocument": {
                "uri": (uri.as_str()),
                "languageId": "markdown",
                "version": 1,
                "text": "+++\ntitle = \"Draft\"\n+++\n\n# Draft\n",
            },
        }),
    );

    let pages = client.list_pages();
    let routes = page_routes(&pages);

    assert!(routes.iter().any(|route| route == "/draft"));
}

pub async fn lsp_reports_diagnostics_and_code_actions_over_stdio() {
    let site = LspSite::new();
    site.write(
        "content/broken.md",
        "# Broken\n\nSee [missing](/missing).\n",
    );
    let mut client = site.client();
    let uri = site.uri("content/broken.md");

    client.notify(
        "textDocument/didOpen",
        value!({
            "textDocument": {
                "uri": (uri.as_str()),
                "languageId": "markdown",
                "version": 1,
                "text": "# Broken\n\nSee [missing](/missing).\n",
            },
        }),
    );

    let diagnostics = client.wait_for_notification("textDocument/publishDiagnostics", |params| {
        params.get("uri").and_then(LspJsonValue::as_str) == Some(uri.as_str())
            && params
                .get("diagnostics")
                .and_then(Value::as_array)
                .is_some_and(|diagnostics| !diagnostics.is_empty())
    });
    let messages = diagnostics
        .get("diagnostics")
        .and_then(Value::as_array)
        .expect("diagnostics array")
        .iter()
        .filter_map(|diagnostic| diagnostic.get("message").and_then(LspJsonValue::as_str))
        .collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| message.contains("/missing")),
        "expected missing route diagnostic, got {messages:?}"
    );

    let actions = client.request(
        "textDocument/codeAction",
        value!({
            "textDocument": { "uri": (uri.as_str()) },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 0 },
            },
            "context": { "diagnostics": [] },
        }),
    );

    let titles = actions
        .as_array()
        .expect("code action array")
        .iter()
        .filter_map(|action| action.get("title").and_then(LspJsonValue::as_str))
        .collect::<Vec<_>>();
    assert!(
        titles.contains(&"Create frontmatter"),
        "expected Create frontmatter action, got {titles:?}"
    );
}

pub async fn lsp_updates_workspace_from_watched_file_changes() {
    let site = LspSite::new();
    let mut client = site.client();
    site.write(
        "content/from-disk.md",
        "+++\ntitle = \"From Disk\"\n+++\n\n# From Disk\n",
    );
    let uri = site.uri("content/from-disk.md");

    client.notify(
        "workspace/didChangeWatchedFiles",
        value!({
            "changes": [{
                "uri": (uri.as_str()),
                "type": 1,
            }],
        }),
    );

    let pages = client.list_pages();
    let routes = page_routes(&pages);

    assert!(routes.iter().any(|route| route == "/from-disk"));
}
