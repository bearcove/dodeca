use super::*;

/// Part 8.1: A request made immediately after FD passing must succeed.
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
