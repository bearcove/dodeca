//! The vendored CMU Pronouncing Dictionary word set.
//!
//! Used to tell *common English words* (which shouldn't auto-link just because a
//! page happens to share their name — `leads`, `hosts`, `ledger`) from
//! *distinctive terms* (project names and jargon coined here — `bearcove`,
//! `dodeca`). The word list (lowercase, deduped, pronunciations stripped) is
//! embedded brotli-compressed and decompressed once on first use.
//!
//! Source: CMUdict, BSD-2-Clause — see `assets/cmudict.LICENSE`.

use std::collections::HashSet;
use std::io::Read;
use std::sync::LazyLock;

/// Brotli-compressed, newline-delimited lowercase word list (~126k words).
static COMPRESSED: &[u8] = include_bytes!("../assets/cmudict-words.txt.br");

static WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let mut text = String::new();
    brotli::Decompressor::new(COMPRESSED, 4096)
        .read_to_string(&mut text)
        .expect("decompress vendored cmudict word list");
    // Leak once: the dictionary lives for the whole process, so leaking lets the
    // set hold `&'static str` slices instead of allocating a String per word.
    let text: &'static str = Box::leak(text.into_boxed_str());
    text.lines().filter(|line| !line.is_empty()).collect()
});

/// Whether `word` is a common English word (case-insensitive). Distinctive
/// terms — project names, jargon, neologisms — return `false`.
pub fn is_common_word(word: &str) -> bool {
    WORDS.contains(word.to_ascii_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_common_words_from_jargon() {
        // Common words a page might be titled after — must not auto-link bare.
        for w in ["leads", "hosts", "ledger", "thesis", "methodology", "helix"] {
            assert!(is_common_word(w), "{w} should be a common word");
        }
        // Case-insensitive.
        assert!(is_common_word("Ledger"));
        // Coined-here terms — distinctive, so they auto-link.
        for w in ["bearcove", "dodeca"] {
            assert!(!is_common_word(w), "{w} should be distinctive");
        }
    }
}
