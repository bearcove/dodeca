#![allow(dead_code)]
#![allow(unused_imports)]

use std::sync::Once;

use hotmeal::Patch;

// Re-export thrall's browser control functions
pub use thrall::{ApplyPatchesResult, ComputeAndApplyResult, DomNode, OwnedPatches};
pub use thrall::{apply_patches, compute_and_apply_patches, parse_to_dom};

static THRALL_INIT: Once = Once::new();

/// Initialize thrall with quiet config for fuzzing (call once at startup).
pub fn init_thrall_quiet() {
    THRALL_INIT.call_once(|| {
        thrall::configure(thrall::ThrallConfig::quiet());
    });
}

mod dom_generator;
pub use dom_generator::*;

mod dom_node;
pub use dom_node::*;

mod patch_trace;
pub use patch_trace::*;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::time::Uptime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Split raw bytes at 0xFF delimiter into two HTML strings.
fn split_input(data: &[u8]) -> Option<(String, String)> {
    let pos = data.iter().position(|&b| b == 0xFF)?;
    let html_a = std::str::from_utf8(&data[..pos]).ok()?.to_owned();
    let html_b = std::str::from_utf8(&data[pos + 1..]).ok()?.to_owned();
    Some((html_a, html_b))
}

/// Check if a newline character reference appears immediately after <textarea>, <pre>, or <listing>.
/// Returns true if such a pattern is found.
fn has_newline_charref_after_raw_text_tag(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();

    // Tags that strip leading newline per HTML5 spec
    for tag in ["<textarea", "<pre", "<listing"] {
        let mut search_pos = 0;
        while let Some(tag_pos) = lower[search_pos..].find(tag) {
            let abs_pos = search_pos + tag_pos;
            let after_tag_name = &html[abs_pos + tag.len()..];

            // Find the end of the start tag (the closing >)
            // Handle both <textarea> and <textarea attr="val">
            if let Some(gt_pos) = after_tag_name.find('>') {
                let content_start = &html[abs_pos + tag.len() + gt_pos + 1..];

                // Check for newline character references
                if is_newline_charref(content_start) {
                    return true;
                }
            }
            search_pos = abs_pos + tag.len();
        }
    }
    false
}

/// Check if the string starts with a character reference that resolves to newline (U+000A).
fn is_newline_charref(s: &str) -> bool {
    if !s.starts_with("&#") {
        return false;
    }
    let after_amp_hash = &s[2..];

    // Hex: &#x0*[aA] or &#X0*[aA]
    if after_amp_hash.starts_with('x') || after_amp_hash.starts_with('X') {
        let hex_part = &after_amp_hash[1..];
        let stripped = hex_part.trim_start_matches('0');
        return stripped.starts_with('a') || stripped.starts_with('A');
    }

    // Decimal: &#0*10
    let stripped = after_amp_hash.trim_start_matches('0');
    stripped.starts_with("10")
        && !stripped[2..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
}

/// Check if an HTML string should be skipped due to known differences.
/// Returns true if the string is valid for browser parity testing.
///
/// Filters are categorized as:
/// - [INVALID]: Invalid inputs that shouldn't be tested
/// - [BROWSER BUG]: Browser deviates from spec, html5ever is correct
/// - [HTML5EVER BUG]: html5ever deviates from spec, browser is correct
/// - [UNCLEAR]: Needs investigation, unclear which is correct
fn is_valid_for_browser_parity(html: &str) -> bool {
    // =========================================================================
    // [INVALID] - Not meaningful test inputs
    // =========================================================================

    // Skip empty inputs
    if html.is_empty() {
        return false;
    }

    // Skip inputs with null bytes
    if html.contains('\0') {
        return false;
    }

    // =========================================================================
    // [BROWSER BUG] - Browser deviates from spec, html5ever is correct
    // =========================================================================

    // Skip inputs starting with whitespace - browsers normalize leading whitespace
    // in innerHTML differently than html5ever's fragment parsing
    if html.starts_with(char::is_whitespace) {
        return false;
    }

    // Skip inputs containing BOM (Byte Order Mark, U+FEFF)
    // html5ever strips leading BOM per spec, but browsers preserve it as text in innerHTML
    if html.contains('\u{feff}') {
        return false;
    }

    // Skip inputs containing CR (\r) - Chrome has numerous bugs with CR normalization
    // in various contexts (LF-CR, CR-CR, /\r, space-CR, etc.). Per INFRA spec, CR should
    // be normalized to LF, but Chrome handles it incorrectly in many edge cases.
    // Firefox and Safari are spec-compliant, html5ever is spec-compliant.
    // See: https://infra.spec.whatwg.org/#normalize-newlines
    if html.contains('\r') {
        return false;
    }

    // Skip inputs ending with incomplete/bogus markup at EOF.
    // Chrome discards bogus comments (<!D, <!D>, etc.) when they appear at EOF
    // with nothing after them, but html5ever correctly preserves them per spec.
    // This affects: unclosed comments, unclosed tags, bogus comments at end.
    // Pattern: ends with <! or <!-- without proper closing, or ends with < followed by text
    {
        let trimmed = html.trim_end();
        // Check for unclosed markup declaration (<!...)
        if let Some(last_lt) = trimmed.rfind('<') {
            let after_lt = &trimmed[last_lt..];
            // Unclosed bogus comment: <! followed by anything except proper comment close
            if after_lt.starts_with("<!") && !after_lt.ends_with("-->") {
                return false;
            }
            // Unclosed tag at very end
            if !after_lt.contains('>') {
                return false;
            }
        }
    }

    // Skip inputs containing DOCTYPE-like bogus comments.
    // Chrome treats <!D, <!DO, <!DOC, <!Da, etc. (anything starting with <!D, case-insensitive)
    // as malformed DOCTYPEs and discards them entirely, while html5ever correctly treats them
    // as bogus comments per spec. This is a known Chrome deviation.
    // Example: "<!D>" becomes nothing in Chrome but "<!--D-->" in html5ever
    // Example: "<!Da>" becomes nothing in Chrome but "<!--Da-->" in html5ever
    // Note: We already filter <!doctype above, this catches partial/malformed cases.
    if html.to_ascii_lowercase().contains("<!d") {
        return false;
    }

    // =========================================================================
    // [HTML5EVER BUG] - html5ever deviates from spec, browser is correct
    // These should be reported upstream to html5ever
    // =========================================================================

    // Skip inputs with <title> inside <svg> - html5ever treats <title> as raw text
    // (HTML parsing mode) even inside SVG, but browsers correctly treat it as a regular
    // SVG element that can contain children.
    // TODO: Report to html5ever - SVG <title> should not be raw text mode
    {
        let lower = html.to_ascii_lowercase();
        if let Some(svg_pos) = lower.find("<svg") {
            if let Some(title_pos) = lower.find("<title") {
                // <title> after <svg> suggests it's inside the SVG
                if title_pos > svg_pos {
                    return false;
                }
            }
        }
    }

    // Skip inputs with <html> inside <math> or <svg> (foreign content).
    // html5ever strips attributes from nested <html> in foreign content, browsers preserve them.
    // TODO: Report to html5ever - nested <html> attrs should be preserved in foreign content
    {
        let lower = html.to_ascii_lowercase();
        let foreign_start = lower.find("<math").or_else(|| lower.find("<svg"));
        if let Some(foreign_pos) = foreign_start {
            if let Some(html_pos) = lower[foreign_pos..].find("<html") {
                if html_pos > 0 {
                    return false;
                }
            }
        }
    }

    // Skip inputs containing <details> - the adoption agency algorithm for formatting
    // elements (like <i>, <b>, <a>) behaves differently at <details> boundaries.
    // html5ever doesn't reopen formatting elements after </details>, browsers do.
    // TODO: Report to html5ever - adoption agency should reopen formatting after </details>
    if html.to_ascii_lowercase().contains("<details") {
        return false;
    }

    // Skip <textarea>, <pre>, or <listing> with leading newline from character reference.
    // Per HTML5 spec, the first LF character token after these start tags should be
    // stripped. This applies even when the LF comes from a character reference like &#xa
    // or &#10. Browsers correctly strip it, but html5ever doesn't.
    // TODO: Report to html5ever - should strip first LF from char ref after textarea/pre/listing
    if has_newline_charref_after_raw_text_tag(html) {
        return false;
    }

    // =========================================================================
    // [UNCLEAR] - Needs investigation
    // =========================================================================

    // Skip inputs containing <html with attributes - browsers have complex normalization
    // rules for <html> attributes (stripping invalid values, certain characters, etc.)
    // that differ from html5ever's permissive parsing.
    {
        let lower = html.to_ascii_lowercase();
        if let Some(pos) = lower.find("<html") {
            let after = &lower[pos + 5..];
            // If there's anything other than > or whitespace+> immediately after <html,
            // it likely has attributes which cause divergent behavior
            if !after.starts_with('>') && !after.trim_start().starts_with('>') {
                return false;
            }
        }
    }

    // Skip inputs containing <template> - template content lives in a DocumentFragment
    // which isn't visible via innerHTML, causing tree structure mismatches
    if html.to_ascii_lowercase().contains("<template") {
        return false;
    }

    // Skip inputs containing <select> - select has special parsing rules for child elements
    // that differ between html5ever and browser innerHTML parsing
    if html.to_ascii_lowercase().contains("<select") {
        return false;
    }

    // Skip inputs containing DOCTYPE in body context - browsers handle bogus DOCTYPE
    // differently than html5ever in fragment parsing
    if html.to_ascii_lowercase().contains("<!doctype") {
        return false;
    }

    // Skip inputs with sequences that create malformed tag names (e.g., "<a<b")
    // These parse differently between html5ever and browsers
    // Pattern: < followed by text, then another < before >
    {
        let mut in_tag = false;
        let mut tag_has_lt = false;
        for c in html.chars() {
            match c {
                '<' if !in_tag => {
                    in_tag = true;
                    tag_has_lt = false;
                }
                '<' if in_tag => {
                    tag_has_lt = true;
                }
                '>' if in_tag => {
                    if tag_has_lt {
                        return false;
                    }
                    in_tag = false;
                }
                _ => {}
            }
        }
    }

    // Skip inputs containing C0 control characters (0x01-0x1F except tab and LF)
    // There are complex structural differences between html5ever and Chrome involving
    // control characters in edge cases. Form feed (0x0C) causes tree structure differences.
    if html
        .bytes()
        .any(|b| matches!(b, 0x01..=0x08 | 0x0B..=0x0C | 0x0E..=0x1F))
    {
        return false;
    }

    true
}

/// Prepare a single HTML string for browser parity testing.
/// Returns raw body content (no DOCTYPE) for fragment parsing.
/// Returns None if the input should be skipped.
pub fn prepare_single_html_input(data: &[u8]) -> Option<String> {
    let html = std::str::from_utf8(data).ok()?;

    if !is_valid_for_browser_parity(html) {
        return None;
    }

    // Return raw content - will be parsed as body fragment
    Some(html.to_string())
}

/// Splits the input data into two HTML fragments for parity testing.
/// Returns raw body content (no DOCTYPE) for fragment parsing.
pub fn prepare_html_inputs(data: &[u8]) -> Option<(String, String)> {
    // Split at 0xFF delimiter
    let (html_a, html_b) = split_input(data)?;

    if !is_valid_for_browser_parity(&html_a) || !is_valid_for_browser_parity(&html_b) {
        return None;
    }

    // Return raw content - will be parsed as body fragments
    Some((html_a, html_b))
}

/// Check if an attribute name is valid for the DOM setAttribute API.
fn is_valid_attr_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('=')
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.starts_with(char::is_whitespace)
        && !name.contains(char::is_control)
}

/// Check if a tag name is valid for the DOM createElement API.
fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('<')
        && !name.contains('>')
        && !name.contains('=')
        && !name.contains('"')
        && !name.contains('\'')
        && !name.contains('/')
        && !name.contains(char::is_whitespace)
        && !name.contains(char::is_control)
}

/// Check if any patch contains attributes with invalid names.
fn patches_have_invalid_attrs(patches: &[Patch]) -> bool {
    for patch in patches {
        match patch {
            Patch::InsertElement {
                attrs, children, ..
            } => {
                for attr in attrs {
                    if !is_valid_attr_name(&attr.name.local) {
                        return true;
                    }
                }
                if insert_contents_have_invalid_attrs(children) {
                    return true;
                }
            }
            Patch::SetAttribute { name, .. } | Patch::RemoveAttribute { name, .. } => {
                if !is_valid_attr_name(&name.local) {
                    return true;
                }
            }
            Patch::UpdateProps { changes, .. } => {
                for change in changes {
                    if let hotmeal::PropKey::Attr(ref qn) = change.name {
                        if !is_valid_attr_name(&qn.local) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn insert_contents_have_invalid_attrs(contents: &[hotmeal::InsertContent]) -> bool {
    for content in contents {
        if let hotmeal::InsertContent::Element {
            attrs, children, ..
        } = content
        {
            for attr in attrs {
                if !is_valid_attr_name(&attr.name.local) {
                    return true;
                }
            }
            if insert_contents_have_invalid_attrs(children) {
                return true;
            }
        }
    }
    false
}

/// Check if any patch contains tags with invalid names.
fn patches_have_invalid_tags(patches: &[Patch]) -> bool {
    for patch in patches {
        if let Patch::InsertElement { tag, children, .. } = patch {
            if !is_valid_tag_name(tag) {
                return true;
            }
            if insert_contents_have_invalid_tags(children) {
                return true;
            }
        }
    }
    false
}

fn insert_contents_have_invalid_tags(contents: &[hotmeal::InsertContent]) -> bool {
    for content in contents {
        if let hotmeal::InsertContent::Element { tag, children, .. } = content {
            if !is_valid_tag_name(tag) {
                return true;
            }
            if insert_contents_have_invalid_tags(children) {
                return true;
            }
        }
    }
    false
}

/// Check if patches are valid for DOM APIs (no invalid attr/tag names).
pub fn patches_are_valid_for_dom(patches: &[Patch]) -> bool {
    !patches_have_invalid_attrs(patches) && !patches_have_invalid_tags(patches)
}

pub fn setup_tracing() {
    let verbosity = color_backtrace::Verbosity::Minimal;

    // Install color-backtrace for better panic output (with forced backtraces and colors)
    color_backtrace::BacktracePrinter::new()
        .verbosity(verbosity)
        .add_frame_filter(Box::new(|frames| {
            frames.retain(|frame| {
                let dominated_by_noise = |name: &str| {
                    // Test harness internals
                    name.starts_with("test::run_test")
                        || name.starts_with("test::__rust_begin_short_backtrace")
                        // Panic/unwind machinery
                        || name.starts_with("std::panicking::")
                        || name.starts_with("std::panic::")
                        || name.starts_with("core::panicking::")
                        // Thread spawning
                        || name.starts_with("std::thread::Builder::spawn_unchecked_")
                        || name.starts_with("std::sys::thread::")
                        || name.starts_with("std::sys::backtrace::")
                        // FnOnce::call_once trampolines in std/core/alloc
                        || name.starts_with("core::ops::function::FnOnce::call_once")
                        || name.starts_with("<alloc::boxed::Box<F,A> as core::ops::function::FnOnce<Args>>::call_once")
                        // AssertUnwindSafe wrapper
                        || name.starts_with("<core::panic::unwind_safe::AssertUnwindSafe<F> as core::ops::function::FnOnce<()>>::call_once")
                        // Low-level threading primitives
                        || name.starts_with("__pthread")
                };
                match &frame.name {
                    Some(name) => !dominated_by_noise(name),
                    None => true,
                }
            })
        }))
        .install(Box::new(termcolor::StandardStream::stderr(
            termcolor::ColorChoice::AlwaysAnsi,
        )));

    // Only install tracing if FUZZ_LOG is explicitly set
    match std::env::var("FUZZ_LOG")
        .ok()
        .and_then(|s| s.parse::<Targets>().ok())
    {
        Some(filter) => {
            eprintln!("Tracing enabled via FUZZ_LOG");
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(true)
                        .with_timer(Uptime::default())
                        .with_target(true)
                        .with_level(true)
                        .with_file(true)
                        .with_line_number(true)
                        .compact(),
                )
                .with(filter)
                .try_init()
                .ok();
        }
        None => {
            eprintln!("Tracing disabled (set FUZZ_LOG=debug to enable)");
        }
    }
}
