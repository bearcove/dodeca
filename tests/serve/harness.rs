//! Test harness for dodeca serve integration tests
//!
//! Provides high-level APIs for testing the server without boilerplate.

use regex::Regex;
use reqwest::blocking::Client;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A running test site with a server and isolated fixture directory
pub struct TestSite {
    child: Child,
    port: u16,
    fixture_dir: PathBuf,
    _temp_dir: tempfile::TempDir,
    client: Client,
}

impl TestSite {
    /// Create a new test site from a fixture directory name
    pub fn new(fixture_name: &str) -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = manifest_dir.join("tests/fixtures").join(fixture_name);
        Self::from_source(&src)
    }

    /// Create a new test site from an arbitrary source directory
    pub fn from_source(src: &Path) -> Self {
        // Create isolated temp directory
        let temp_dir = tempfile::Builder::new()
            .prefix("dodeca-test-")
            .tempdir()
            .expect("create temp dir");

        let fixture_dir = temp_dir.path().to_path_buf();
        copy_dir_recursive(src, &fixture_dir).expect("copy fixture");

        // Ensure .cache exists and is empty
        let cache_dir = fixture_dir.join(".cache");
        let _ = fs::remove_dir_all(&cache_dir);
        fs::create_dir_all(&cache_dir).expect("create cache dir");

        // Start server
        let fixture_str = fixture_dir.to_string_lossy().to_string();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ddc"))
            .args(["serve", &fixture_str, "--no-tui", "-p", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start server");

        // Wait for LISTENING_PORT
        let stdout = child.stdout.take().expect("capture stdout");
        let mut reader = BufReader::new(stdout);
        let port_re = Regex::new(r"LISTENING_PORT=(\d+)").unwrap();
        let mut port = None;

        for line in reader.by_ref().lines() {
            let line = line.expect("read line");
            if let Some(caps) = port_re.captures(&line) {
                port = Some(caps[1].parse::<u16>().expect("parse port"));
                break;
            }
        }

        let port = match port {
            Some(p) => p,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Server did not print LISTENING_PORT");
            }
        };

        // Drain stdout in background
        std::thread::spawn(move || {
            for line in reader.lines() {
                if let Ok(line) = line {
                    eprintln!("[server] {line}");
                } else {
                    break;
                }
            }
        });

        // Wait for server to be ready
        let client = Client::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if client
                .get(format!("http://127.0.0.1:{}/", port))
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Self {
            child,
            port,
            fixture_dir,
            _temp_dir: temp_dir,
            client,
        }
    }

    /// Get the base URL for this server
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    /// Make a GET request and return a Response helper
    pub fn get(&self, path: &str) -> Response {
        let url = self.url(path);
        let resp = self
            .client
            .get(&url)
            .send()
            .unwrap_or_else(|e| panic!("GET {url} failed: {e}"));
        Response {
            status: resp.status().as_u16(),
            body: resp.text().unwrap_or_default(),
        }
    }

    /// Wait for a path to return 200, retrying until timeout
    pub fn wait_for(&self, path: &str, timeout: Duration) -> Response {
        let url = self.url(path);
        let deadline = Instant::now() + timeout;
        let mut last_status = None;

        while Instant::now() < deadline {
            match self.client.get(&url).send() {
                Ok(resp) if resp.status().is_success() => {
                    return Response {
                        status: resp.status().as_u16(),
                        body: resp.text().unwrap_or_default(),
                    };
                }
                Ok(resp) => {
                    last_status = Some(resp.status().as_u16());
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!(
            "GET {} did not return 200 within {:?}, last status: {:?}",
            path, timeout, last_status
        );
    }

    /// Read a file from the fixture directory
    pub fn read_file(&self, rel_path: &str) -> String {
        let path = self.fixture_dir.join(rel_path);
        fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Write a file to the fixture directory
    pub fn write_file(&self, rel_path: &str, content: &str) {
        let path = self.fixture_dir.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, content)
            .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
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
        fs::remove_file(&path)
            .unwrap_or_else(|e| panic!("delete {}: {e}", path.display()));
    }

    /// Wait until a condition is true, polling at intervals
    /// Returns the value produced by the condition, or panics on timeout
    pub fn wait_until<T, F>(&self, timeout: Duration, mut condition: F) -> T
    where
        F: FnMut() -> Option<T>,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(result) = condition() {
                return result;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("Condition not met within {:?}", timeout);
    }

    /// Wait for the file watcher debounce window
    pub fn wait_debounce(&self) {
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Get the fixture directory path
    #[allow(dead_code)]
    pub fn fixture_dir(&self) -> &Path {
        &self.fixture_dir
    }
}

impl Drop for TestSite {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// HTTP response helper
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    /// Assert status code
    pub fn assert_status(&self, expected: u16) -> &Self {
        assert_eq!(
            self.status, expected,
            "Expected status {expected}, got {}",
            self.status
        );
        self
    }

    /// Assert status is 200 OK
    pub fn assert_ok(&self) -> &Self {
        self.assert_status(200)
    }

    /// Assert status is 404 Not Found
    #[allow(dead_code)]
    pub fn assert_not_found(&self) -> &Self {
        self.assert_status(404)
    }

    /// Get the response body as text
    pub fn text(&self) -> &str {
        &self.body
    }

    /// Assert body contains a substring
    pub fn assert_contains(&self, needle: &str) -> &Self {
        assert!(
            self.body.contains(needle),
            "Response body should contain '{needle}'.\nBody:\n{}",
            truncate(&self.body, 500)
        );
        self
    }

    /// Assert body does NOT contain a substring
    pub fn assert_not_contains(&self, needle: &str) -> &Self {
        assert!(
            !self.body.contains(needle),
            "Response body should NOT contain '{needle}'.\nBody:\n{}",
            truncate(&self.body, 500)
        );
        self
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

    /// Find a CSS link matching a pattern (e.g., "/css/style.*.css")
    /// Handles both quoted (href="/path") and unquoted (href=/path) attributes
    pub fn css_link(&self, pattern: &str) -> Option<String> {
        // For minified HTML without quotes, use word-boundary matching
        let escaped = regex::escape(pattern).replace(r"\*", r#"[^"'\s>]+"#);
        let re_str = format!(r#"href=["']?({escaped})["']?"#);
        self.extract(&re_str)
    }

    /// Find a JS script src matching a pattern
    /// Handles both quoted and unquoted attributes
    #[allow(dead_code)]
    pub fn script_src(&self, pattern: &str) -> Option<String> {
        let escaped = regex::escape(pattern).replace(r"\*", r#"[^"'\s>]+"#);
        let re_str = format!(r#"src=["']?({escaped})["']?"#);
        self.extract(&re_str)
    }

    /// Find an image src matching a pattern
    /// Handles both quoted and unquoted attributes
    pub fn img_src(&self, pattern: &str) -> Option<String> {
        let escaped = regex::escape(pattern).replace(r"\*", r#"[^"'\s>]+"#);
        let re_str = format!(r#"<img[^>]+src=["']?({escaped})["']?"#);
        self.extract(&re_str)
    }

    /// Check if response contains a <picture> element for an image
    #[allow(dead_code)]
    pub fn has_picture_element(&self, img_pattern: &str) -> bool {
        let escaped = regex::escape(img_pattern).replace(r"\*", r#"[^"']+"#);
        let re_str = format!(r#"<picture>.*?{escaped}.*?</picture>"#);
        Regex::new(&re_str)
            .map(|re| re.is_match(&self.body))
            .unwrap_or(false)
    }
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
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

/// Truncate a string for display
fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
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
    pub fn assert_failure(&self) -> &Self {
        assert!(
            !self.success,
            "Build should have failed but succeeded.\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
        );
        self
    }

    /// Assert combined output contains a substring
    pub fn assert_output_contains(&self, needle: &str) -> &Self {
        let combined = format!("{}{}", self.stdout, self.stderr);
        assert!(
            combined.contains(needle),
            "output should contain '{needle}'.\nstdout:\n{}\nstderr:\n{}",
            self.stdout, self.stderr
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
    let output = Command::new(env!("CARGO_BIN_EXE_ddc"))
        .args(["build", &fixture_str])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run ddc build");

    BuildResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// Build a site from inline content (creates minimal fixture on the fly)
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
        fs::write(fixture_dir.join("sass/main.scss"), "body { margin: 0; }")
            .expect("write sass");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_extract() {
        let resp = Response {
            status: 200,
            body: r#"<link href="/css/style.abc123.css" rel="stylesheet">"#.to_string(),
        };
        assert_eq!(
            resp.css_link("/css/style.*.css"),
            Some("/css/style.abc123.css".to_string())
        );
    }
}
