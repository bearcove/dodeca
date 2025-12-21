use super::*;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;

/// Part 8.1: When DODECA_CELL_PATH points to a directory missing ddc-cell-http,
/// connections must NOT get refused/reset. The server should accept the connection
/// and respond with HTTP 500 (or similar boot-fatal response).
///
/// This verifies that:
/// 1. The accept loop is never aborted on cell loading failures
/// 2. Connections are held until boot state is determined
/// 3. HTTP 500 is returned instead of connection reset
pub fn missing_cell_returns_http_500_not_connection_reset() {
    // Create a site with an empty cell path (no cells available)
    let site = TestSite::with_empty_cell_path("sample-site");

    // Give the server a moment to start and reach its Fatal boot state
    std::thread::sleep(Duration::from_millis(500));

    // Use raw TCP to verify we can connect and get a response (not ECONNREFUSED/ECONNRESET)
    let addr = format!("127.0.0.1:{}", site.port);

    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            panic!(
                "Connection should succeed even with missing cells, got: {} (kind={:?})",
                e,
                e.kind()
            );
        }
    };

    // Send a simple HTTP request
    let request = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        site.port
    );
    stream
        .write_all(request.as_bytes())
        .expect("write should succeed");

    // Read response with timeout
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);

    let response_str = String::from_utf8_lossy(&response);

    // Verify we got an HTTP response (not empty/connection reset)
    assert!(
        !response.is_empty(),
        "Should get an HTTP response, not connection reset"
    );

    // Should be HTTP 500 (boot-fatal response)
    assert!(
        response_str.starts_with("HTTP/1.1 500"),
        "Expected HTTP 500 response, got: {}",
        response_str.lines().next().unwrap_or("<empty>")
    );
}

/// Part 8.2: A request made immediately after FD passing must succeed.
/// The connection should stay open while the server boots, and complete
/// once the revision is ready.
///
/// This verifies that:
/// 1. The accept loop starts accepting immediately
/// 2. Connection handlers wait for boot to complete
/// 3. Requests succeed after boot completes
pub fn immediate_request_after_fd_pass_succeeds() {
    // Normal site with all cells - the server should boot successfully
    let site = TestSite::new("sample-site");

    // Make a request immediately - should succeed even if server is still booting
    let resp = site.get("/");

    // The request should succeed (200 OK)
    resp.assert_ok();

    // And should have real content
    resp.assert_contains("<!DOCTYPE html>");
}
