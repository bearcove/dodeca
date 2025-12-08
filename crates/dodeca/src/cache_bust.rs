//! Cache busting for static assets
//!
//! Hashes file content and embeds in filename for optimal browser caching.
//! Uses the "dodeca alphabet" - base12 encoding with characters from "dodeca"
//! (a, c, d, e, o) plus digits 0-6: `0123456acdeo`
//!
//! Example: `main.css` → `main.0a3dec24oc21.css`

use rapidhash::fast::RapidHasher;
use std::hash::Hasher;

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
}
