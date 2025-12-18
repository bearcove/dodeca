use super::harness::TestSite;

/// Verifies that repeated navigations reuse Picante cache across per-request snapshots.
///
/// We assert this by checking that the second request produces far fewer (ideally zero)
/// `picante` "compute: start" log lines than the first request.
#[test_log::test]
fn navigating_twice_should_not_recompute_queries() {
    // Ensure we see picante internal logs.
    unsafe { std::env::set_var("RUST_LOG", "debug") };

    let site = TestSite::new("sample-site");

    // Discard startup logs; we're measuring request-induced work.
    site.clear_logs();

    site.get("/guide/").assert_ok();
    let cursor = site.log_cursor();

    let first_compute_starts = site.count_logs_since(0, "compute: start");

    site.get("/guide/").assert_ok();
    let second_compute_starts = site.count_logs_since(cursor, "compute: start");

    // We expect the second request to be almost entirely cache hits.
    // Allow a small number of stragglers for nondeterminism (e.g. background tasks),
    // but it should be dramatically smaller than the first.
    assert!(
        second_compute_starts <= 2 || second_compute_starts * 10 < first_compute_starts.max(1),
        "expected second navigation to trigger far fewer computations: first={first_compute_starts}, second={second_compute_starts}"
    );
}
