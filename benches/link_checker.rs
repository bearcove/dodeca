//! Benchmarks for link checking
//!
//! Run with: cargo bench --bench link_checker

use divan::{black_box, Bencher};
use dodeca::link_checker::{extract_links, check_internal_links, Page};
use dodeca::types::Route;

fn main() {
    divan::main();
}

// ============================================================================
// HTML generators for link checking
// ============================================================================

/// Generate HTML for a page with many internal links
fn generate_page_with_links(num_links: usize, page_id: usize) -> String {
    let mut html = String::from("<!DOCTYPE html><html><head><title>Test</title></head><body>");
    html.push_str("<nav>");

    // Add navigation links
    for i in 0..10 {
        html.push_str(&format!(r#"<a href="/page/{}"">Page {}</a>"#, i, i));
    }
    html.push_str("</nav>");

    html.push_str("<main>");
    // Add content links
    for i in 0..num_links {
        let target = (page_id + i) % 100; // Create links to various pages
        html.push_str(&format!(
            r#"<p>This is paragraph {} with a <a href="/page/{}">link to page {}</a> and some text.</p>"#,
            i, target, target
        ));

        // Add some external links too
        if i % 5 == 0 {
            html.push_str(&format!(
                r#"<p>Check out <a href="https://example.com/resource/{}">this external resource</a>.</p>"#,
                i
            ));
        }
    }
    html.push_str("</main>");

    html.push_str("</body></html>");
    html
}

/// Generate a small site (10 pages)
fn generate_small_site() -> Vec<(String, String)> {
    (0..10)
        .map(|i| {
            let route = format!("/page/{}", i);
            let html = generate_page_with_links(20, i);
            (route, html)
        })
        .collect()
}

/// Generate a medium site (100 pages)
fn generate_medium_site() -> Vec<(String, String)> {
    (0..100)
        .map(|i| {
            let route = format!("/page/{}", i);
            let html = generate_page_with_links(50, i);
            (route, html)
        })
        .collect()
}

/// Generate a large site (1000 pages)
fn generate_large_site() -> Vec<(String, String)> {
    (0..1000)
        .map(|i| {
            let route = format!("/page/{}", i);
            let html = generate_page_with_links(30, i);
            (route, html)
        })
        .collect()
}

// ============================================================================
// Link extraction benchmarks
// ============================================================================

/// Helper struct to hold route and html together with proper lifetimes
struct SitePage {
    route: Route,
    html: String,
}

#[divan::bench]
fn extract_links_small_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_small_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    bencher.bench(|| {
        let pages_iter = site.iter().map(|p| Page {
            route: &p.route,
            html: &p.html,
        });
        black_box(extract_links(pages_iter))
    });
}

#[divan::bench]
fn extract_links_medium_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_medium_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    bencher.bench(|| {
        let pages_iter = site.iter().map(|p| Page {
            route: &p.route,
            html: &p.html,
        });
        black_box(extract_links(pages_iter))
    });
}

#[divan::bench]
fn extract_links_large_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_large_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    bencher.bench(|| {
        let pages_iter = site.iter().map(|p| Page {
            route: &p.route,
            html: &p.html,
        });
        black_box(extract_links(pages_iter))
    });
}

// ============================================================================
// Internal link checking benchmarks
// ============================================================================

#[divan::bench]
fn check_internal_small_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_small_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    let pages_iter = site.iter().map(|p| Page {
        route: &p.route,
        html: &p.html,
    });
    let extracted = extract_links(pages_iter);

    bencher.bench(|| black_box(check_internal_links(&extracted)));
}

#[divan::bench]
fn check_internal_medium_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_medium_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    let pages_iter = site.iter().map(|p| Page {
        route: &p.route,
        html: &p.html,
    });
    let extracted = extract_links(pages_iter);

    bencher.bench(|| black_box(check_internal_links(&extracted)));
}

#[divan::bench]
fn check_internal_large_site(bencher: Bencher) {
    let site: Vec<SitePage> = generate_large_site()
        .into_iter()
        .map(|(route, html)| SitePage {
            route: Route::from(route),
            html,
        })
        .collect();

    let pages_iter = site.iter().map(|p| Page {
        route: &p.route,
        html: &p.html,
    });
    let extracted = extract_links(pages_iter);

    bencher.bench(|| black_box(check_internal_links(&extracted)));
}
