//! Integration tests that parse real-world HTML fixture files.
//!
//! These tests verify that the parser can handle complex, real-world HTML
//! without panicking or producing errors.

use hotmeal::parse;
use std::fs;
use std::path::Path;

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Test that we can parse all fixture files without panicking.
#[test]
fn parse_all_fixtures_without_panic() {
    let fixtures = fixtures_dir();
    if !fixtures.exists() {
        eprintln!("Fixtures directory doesn't exist, skipping test");
        return;
    }

    let mut count = 0;
    let mut errors = Vec::new();

    for entry in fs::read_dir(&fixtures).expect("Failed to read fixtures directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "html") {
            count += 1;
            let filename = path.file_name().unwrap().to_string_lossy();
            let content = fs::read_to_string(&path).expect("Failed to read fixture file");

            // Test that parsing doesn't panic
            let result = std::panic::catch_unwind(|| {
                let _ = parse(&content);
            });

            if result.is_err() {
                errors.push(format!("Panic while parsing: {}", filename));
            }
        }
    }

    assert!(count > 0, "No fixture files found in {:?}", fixtures);
    assert!(
        errors.is_empty(),
        "Errors parsing fixtures:\n{}",
        errors.join("\n")
    );
    println!("Successfully parsed {} fixture files", count);
}

/// Test that all fixtures produce a valid Html structure with body.
#[test]
fn all_fixtures_produce_body() {
    let fixtures = fixtures_dir();
    if !fixtures.exists() {
        return;
    }

    let mut count = 0;

    for entry in fs::read_dir(&fixtures).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "html") {
            count += 1;
            let content = fs::read_to_string(&path).unwrap();

            let html = parse(&content);

            // Each file should produce a body
            assert!(
                html.body.is_some(),
                "No body from {}",
                path.file_name().unwrap().to_string_lossy()
            );
        }
    }

    assert!(count > 0);
    println!("All {} fixtures produced body elements", count);
}

/// Test serialization doesn't panic on fixtures.
#[test]
fn fixtures_serialize_no_panic() {
    let fixtures = fixtures_dir();
    if !fixtures.exists() {
        return;
    }

    let mut count = 0;

    for entry in fs::read_dir(&fixtures).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "html") {
            count += 1;
            let content = fs::read_to_string(&path).unwrap();

            // Parse -> serialize should not panic
            let html = parse(&content);
            let result = std::panic::catch_unwind(|| {
                let _ = hotmeal::to_string(&html);
            });

            assert!(
                result.is_ok(),
                "Serialization panicked for {}",
                path.file_name().unwrap().to_string_lossy()
            );
        }
    }

    assert!(count > 0);
    println!("All {} fixtures serialized without panic", count);
}
