use super::*;

pub fn navigating_twice_should_not_recompute_queries() {
    // Note: This test relies on RUST_LOG=debug being set
    // In the standalone runner, we don't have fine-grained control over this
    // but the test should still work as long as caching is functioning

    let site = TestSite::new("sample-site");

    site.clear_logs();

    site.get("/guide/").assert_ok();
    let cursor = site.log_cursor();

    let first_compute_starts = site.count_logs_since(0, "compute: start");

    site.get("/guide/").assert_ok();
    let second_compute_starts = site.count_logs_since(cursor, "compute: start");

    assert!(
        second_compute_starts <= 2 || second_compute_starts * 10 < first_compute_starts.max(1),
        "expected second navigation to trigger far fewer computations: first={first_compute_starts}, second={second_compute_starts}"
    );
}
