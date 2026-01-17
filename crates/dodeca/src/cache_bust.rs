//! Cache busting for static assets
//!
//! Hashes file content and embeds in filename for optimal browser caching.
//! Uses the "dodeca alphabet" - base12 encoding with characters from "dodeca"
//! (a, c, d, e, o) plus digits 0-6: `0123456acdeo`
//!
//! Example: `main.css` → `main.0a3dec24oc21.css`
//!
//! Also detects files that already have cache-busting hashes (e.g. from Vite/Webpack)
//! to avoid double-hashing: `main-B6eUmL6x.js` is left unchanged.

use rapidhash::fast::RapidHasher;
use std::hash::Hasher;

/// Check if a filename already has a cache-busting hash embedded.
/// Detects common patterns from bundlers like Vite, Webpack, Parcel:
/// - `name-HASH.ext` (Vite style: main-B6eUmL6x.js)
/// - `name.HASH.ext` (Webpack style: main.abc123.js)
///
/// Returns true if the filename appears to already be cache-busted.
pub fn has_existing_hash(path: &str) -> bool {
    // Get just the filename without directory
    let filename = path.rsplit('/').next().unwrap_or(path);

    // Must have an extension
    let Some(dot_pos) = filename.rfind('.') else {
        return false;
    };

    let name_part = &filename[..dot_pos];

    // Check for Vite-style: name-HASH (dash followed by 6-12 alphanumeric chars)
    if let Some(dash_pos) = name_part.rfind('-') {
        let potential_hash = &name_part[dash_pos + 1..];
        if is_hash_like(potential_hash) {
            return true;
        }
    }

    // Check for Webpack-style: name.HASH (dot followed by 6-12 alphanumeric chars before extension)
    // e.g., main.abc123.js - need to find second-to-last dot
    if let Some(second_dot) = name_part.rfind('.') {
        let potential_hash = &name_part[second_dot + 1..];
        if is_hash_like(potential_hash) {
            return true;
        }
    }

    false
}

/// Check if a string looks like a bundler-generated hash.
/// Must be 6-12 characters using alphanumeric, dash, or underscore (base64url-like).
fn is_hash_like(s: &str) -> bool {
    let len = s.len();
    (6..=12).contains(&len)
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The dodeca alphabet: 7 digits (0-6) + 5 unique letters from "dodeca"
/// 12 characters for base12 encoding (dodeca = 12-sided polyhedron)
const DODECA_ALPHABET: &[u8; 12] = b"0123456acdeo";

/// Number of characters in dodeca hashes (12 for "dodeca" = 12-sided)
const DODECA_HASH_LEN: usize = 12;

/// Encode a u64 hash as a 12-character dodeca string (base12)
/// 12^12 ≈ 8.9 trillion combinations - plenty of entropy
pub fn encode_dodeca(mut hash: u64) -> String {
    let mut result = [b'0'; DODECA_HASH_LEN];

    for i in (0..DODECA_HASH_LEN).rev() {
        result[i] = DODECA_ALPHABET[(hash % 12) as usize];
        hash /= 12;
    }

    // SAFETY: DODECA_ALPHABET only contains ASCII characters
    unsafe { String::from_utf8_unchecked(result.to_vec()) }
}

/// Generate a short hash from content for cache busting
/// Returns 12 dodeca characters (base12 with dodeca alphabet)
pub fn content_hash(content: &[u8]) -> String {
    let mut hasher = RapidHasher::default();
    hasher.write(content);
    encode_dodeca(hasher.finish())
}

/// Generate cache-busted filename
/// `fonts/Inter.woff2` + hash `a1b2c3d4` → `fonts/Inter.a1b2c3d4.woff2`
pub fn cache_busted_path(path: &str, hash: &str) -> String {
    if let Some(dot_pos) = path.rfind('.') {
        format!("{}.{}{}", &path[..dot_pos], hash, &path[dot_pos..])
    } else {
        format!("{path}.{hash}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_dodeca() {
        // Test that encoding produces valid dodeca alphabet characters
        let encoded = encode_dodeca(0);
        assert_eq!(encoded, "000000000000");
        assert_eq!(encoded.len(), 12);

        // Test with a known value
        let encoded = encode_dodeca(12345678901234567890);
        assert_eq!(encoded.len(), 12);
        // All characters should be from dodeca alphabet (base12: 0-6 + acdeo)
        for c in encoded.chars() {
            assert!(
                "0123456acdeo".contains(c),
                "Invalid character in dodeca hash: {c}"
            );
        }
    }

    #[test]
    fn test_content_hash() {
        let hash1 = content_hash(b"hello world");
        let hash2 = content_hash(b"hello world");
        let hash3 = content_hash(b"different content");

        assert_eq!(hash1.len(), 12); // dodeca = 12 characters
        assert_eq!(hash1, hash2); // deterministic
        assert_ne!(hash1, hash3); // different content = different hash

        // All characters should be from dodeca alphabet (base12: 0-6 + acdeo)
        for c in hash1.chars() {
            assert!(
                "0123456acdeo".contains(c),
                "Invalid character in dodeca hash: {c}"
            );
        }
    }

    #[test]
    fn test_cache_busted_path() {
        assert_eq!(
            cache_busted_path("main.css", "0a3dec47oc21"),
            "main.0a3dec47oc21.css"
        );
        assert_eq!(
            cache_busted_path("fonts/Inter.woff2", "dec0da123456"),
            "fonts/Inter.dec0da123456.woff2"
        );
        assert_eq!(
            cache_busted_path("fonts/Inter-Bold.woff2", "123456789012"),
            "fonts/Inter-Bold.123456789012.woff2"
        );
        assert_eq!(
            cache_busted_path("noextension", "acdeo0123456"),
            "noextension.acdeo0123456"
        );
    }

    #[test]
    fn test_has_existing_hash() {
        // Vite-style hashes (dash + 6-12 alphanumeric chars)
        assert!(has_existing_hash("main-B6eUmL6x.js"));
        assert!(has_existing_hash("monaco/main-B6eUmL6x.js"));
        assert!(has_existing_hash("typescript-Bq0JxXsY.js"));
        assert!(has_existing_hash("clojure-BAuDPsal.js"));

        // Webpack-style hashes (dot + chars + dot + ext)
        assert!(has_existing_hash("main.abc123.js"));
        assert!(has_existing_hash("vendor.1234abcd.css"));

        // Not hashed - should return false
        assert!(!has_existing_hash("main.js"));
        assert!(!has_existing_hash("main.css"));
        assert!(!has_existing_hash("fonts/Inter.woff2"));
        assert!(!has_existing_hash("inter-bold.woff2")); // "bold" is only 4 chars

        // Edge cases
        assert!(!has_existing_hash("noextension"));
        assert!(!has_existing_hash("file-ab.js")); // hash too short (2 chars)
        assert!(!has_existing_hash("file-abc.js")); // hash too short (3 chars)
    }
}
