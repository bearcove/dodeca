//! Inline notes: `<!-- note … -->` HTML comments carrying an embedded
//! frontmatter + markdown document.
//!
//! A note is authored as a block-level HTML comment whose first token is
//! `note`, followed by an optional TOML/YAML frontmatter block and a markdown
//! body:
//!
//! ```text
//! <!-- note
//! +++
//! author = "amos"
//! kind = "question"
//! +++
//! Why is this **clamped** here?
//! -->
//! ```
//!
//! Notes live in the markdown source so they are version-controlled and move
//! with the text they annotate. The renderer either strips them (production) or
//! renders them to an `<aside class="dodeca-note">` (development), driven by
//! [`RenderOptions::render_notes`](crate::RenderOptions::render_notes).

use facet::Facet;

use crate::strip_frontmatter;

/// Metadata parsed from a note's frontmatter.
///
/// All fields are optional: a bare `<!-- note … -->` with only a body is valid.
#[derive(Debug, Clone, Default, Facet)]
pub struct NoteMeta {
    /// Who wrote the note.
    #[facet(default)]
    pub author: Option<String>,

    /// Free-form kind used for styling (e.g. `note`, `question`, `todo`).
    #[facet(default)]
    pub kind: Option<String>,

    /// Thread id linking this note's comments together (replies share the id).
    #[facet(default)]
    pub id: Option<String>,

    /// Creation timestamp (RFC 3339), for rendering a byline date.
    #[facet(default)]
    pub created: Option<String>,

    /// Whether the thread is resolved. Set on the thread's root note; resolved
    /// threads are hidden by default in the dev overlay.
    #[facet(default)]
    pub resolved: Option<bool>,

    /// The rendered text the note anchors to within its block. The highlight is
    /// derived non-destructively from this (the dev overlay locates the quote in
    /// the rendered DOM and wraps it) — the source prose is never modified. A
    /// note without a quote is a block-level note.
    #[facet(default)]
    pub quote: Option<String>,
}

/// A parsed inline note: its metadata plus the raw markdown body.
#[derive(Debug, Clone)]
pub struct Note {
    /// Parsed frontmatter metadata.
    pub meta: NoteMeta,
    /// The markdown body (still unrendered).
    pub body: String,
}

/// Parse the full raw text of an HTML block as a note, if it is one.
///
/// `block` is the verbatim source of an HTML comment, e.g.
/// `"<!-- note\n+++\n…\n+++\nbody\n-->\n"`. Returns `None` for ordinary HTML
/// blocks (including comments whose first token is not `note`), so callers can
/// fall back to their normal raw-HTML handling.
pub fn parse_note(block: &str) -> Option<Note> {
    let inner = block.trim();
    let inner = inner.strip_prefix("<!--")?;
    let inner = inner.strip_suffix("-->")?;
    let inner = inner.trim_start();

    // The first token must be exactly `note` (not e.g. `notebook`).
    let rest = inner.strip_prefix("note")?;
    match rest.chars().next() {
        None => {}                         // `<!-- note -->`
        Some(c) if c.is_whitespace() => {} // `<!-- note\n…`
        Some(_) => return None,            // `<!-- notebook … -->`
    }

    // Skip the whitespace/newline after the `note` keyword. What remains is the
    // embedded document: optional `+++`/`---` frontmatter followed by a body.
    let doc = rest.trim_start();
    let stripped = strip_frontmatter(doc);
    let meta = match (stripped.raw, stripped.format) {
        (Some(raw), Some(crate::FrontmatterFormat::Toml)) => {
            facet_toml::from_str::<NoteMeta>(raw).unwrap_or_default()
        }
        (Some(raw), Some(crate::FrontmatterFormat::Yaml)) => {
            facet_yaml::from_str::<NoteMeta>(raw).unwrap_or_default()
        }
        _ => NoteMeta::default(),
    };

    Some(Note {
        meta,
        body: stripped.body.trim().to_string(),
    })
}

/// Serialize a note to its canonical `<!-- note … -->` comment form.
///
/// Round-trips with [`parse_note`]. Returns `None` when `body` contains the
/// comment terminator `-->`, which cannot be represented inside an HTML comment.
pub fn to_comment(meta: &NoteMeta, body: &str) -> Option<String> {
    if body.contains("-->") {
        return None;
    }

    let mut out = String::from("<!-- note\n");

    // Build an object of only the present fields and serialize that: facet-toml
    // refuses to emit `null`, so serializing `NoteMeta` directly fails whenever a
    // field is `None`. A bare note (no metadata) parses back fine without any
    // frontmatter block.
    let mut obj = facet_value::VObject::new();
    if let Some(author) = &meta.author {
        obj.insert("author", author.as_str());
    }
    if let Some(kind) = &meta.kind {
        obj.insert("kind", kind.as_str());
    }
    if let Some(id) = &meta.id {
        obj.insert("id", id.as_str());
    }
    if let Some(created) = &meta.created {
        obj.insert("created", created.as_str());
    }
    if let Some(resolved) = meta.resolved {
        obj.insert("resolved", resolved);
    }
    if let Some(quote) = &meta.quote {
        obj.insert("quote", quote.as_str());
    }
    if !obj.is_empty()
        && let Ok(fm) = facet_toml::to_string(&obj.into_value())
    {
        let fm = fm.trim();
        if !fm.is_empty() {
            out.push_str("+++\n");
            out.push_str(fm);
            out.push_str("\n+++\n");
        }
    }

    out.push_str(body.trim());
    out.push_str("\n-->");
    Some(out)
}

/// The custom element used to highlight the exact span a note refers to.
///
/// Authored into the markdown source around the annotated text. In development
/// (`render_notes`) it is left in the rendered HTML for the dev overlay's CSS to
/// highlight; in production it is stripped by [`strip_marks`] so it leaves no
/// trace (not even in view-source).
pub const MARK_TAG: &str = "dodeca-mark";

/// Wrap an inline span of markdown source in a note highlight element, linked to
/// its note thread by `id` (emitted as `data-note-id`).
pub fn wrap_mark(inner: &str, id: Option<&str>) -> String {
    match id {
        Some(id) => format!(
            "<{MARK_TAG} data-note-id=\"{}\">{inner}</{MARK_TAG}>",
            attr_escape(id)
        ),
        None => format!("<{MARK_TAG}>{inner}</{MARK_TAG}>"),
    }
}

/// Remove `<dodeca-mark …>` / `</dodeca-mark>` tags (with or without attributes)
/// from rendered HTML, keeping the inner content. Used in production
/// (`render_notes` off) so note highlights leave no trace in the served HTML.
pub fn strip_marks(html: &str) -> String {
    let mut out = html.replace(&format!("</{MARK_TAG}>"), "");
    let open = format!("<{MARK_TAG}");
    while let Some(start) = out.find(&open) {
        // Drop everything from `<dodeca-mark` up to and including the next `>`.
        match out[start..].find('>') {
            Some(rel_end) => out.replace_range(start..start + rel_end + 1, ""),
            None => break,
        }
    }
    out
}

/// Wrap already-rendered body HTML in the note's `<aside>` element.
///
/// Emits `data-kind` / `data-author` attributes (when present) so a stylesheet
/// can theme notes by kind and show a byline.
pub fn render_aside(meta: &NoteMeta, body_html: &str) -> String {
    let mut out = String::from("<aside class=\"dodeca-note\"");
    if let Some(id) = &meta.id {
        out.push_str(&format!(" data-note-id=\"{}\"", attr_escape(id)));
    }
    if let Some(kind) = &meta.kind {
        out.push_str(&format!(" data-kind=\"{}\"", attr_escape(kind)));
    }
    if let Some(author) = &meta.author {
        out.push_str(&format!(" data-author=\"{}\"", attr_escape(author)));
    }
    if let Some(created) = &meta.created {
        out.push_str(&format!(" data-created=\"{}\"", attr_escape(created)));
    }
    if meta.resolved == Some(true) {
        out.push_str(" data-resolved=\"true\"");
    }
    if let Some(quote) = &meta.quote {
        out.push_str(&format!(" data-quote=\"{}\"", attr_escape(quote)));
    }
    out.push('>');
    out.push_str(body_html);
    out.push_str("</aside>");
    out
}

/// Escape a string for use inside a double-quoted HTML attribute value.
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_with_frontmatter() {
        let block =
            "<!-- note\n+++\nauthor = \"amos\"\nkind = \"question\"\n+++\nWhy **clamp** here?\n-->";
        let note = parse_note(block).expect("should parse");
        assert_eq!(note.meta.author.as_deref(), Some("amos"));
        assert_eq!(note.meta.kind.as_deref(), Some("question"));
        assert_eq!(note.body, "Why **clamp** here?");
    }

    #[test]
    fn parses_bare_note_without_frontmatter() {
        let note = parse_note("<!-- note\njust a thought\n-->").expect("should parse");
        assert!(note.meta.author.is_none());
        assert!(note.meta.kind.is_none());
        assert_eq!(note.body, "just a thought");
    }

    #[test]
    fn rejects_non_note_comments() {
        assert!(parse_note("<!-- TODO: fix this -->").is_none());
        assert!(parse_note("<!-- notebook entry -->").is_none());
        assert!(parse_note("<div>not a comment</div>").is_none());
    }

    #[test]
    fn to_comment_round_trips_with_meta() {
        let meta = NoteMeta {
            author: Some("amos".into()),
            kind: Some("question".into()),
            ..Default::default()
        };
        let comment = to_comment(&meta, "Why **clamp** here?").expect("serializable");
        let parsed = parse_note(&comment).expect("round-trips");
        assert_eq!(parsed.meta.author.as_deref(), Some("amos"));
        assert_eq!(parsed.meta.kind.as_deref(), Some("question"));
        assert_eq!(parsed.body, "Why **clamp** here?");
    }

    #[test]
    fn to_comment_round_trips_bare() {
        let comment = to_comment(&NoteMeta::default(), "just a thought").expect("serializable");
        let parsed = parse_note(&comment).expect("round-trips");
        assert!(parsed.meta.author.is_none());
        assert!(parsed.meta.kind.is_none());
        assert_eq!(parsed.body, "just a thought");
    }

    /// A note with only `kind` set (the common case from the overlay, which
    /// never sends an author) must keep its frontmatter — facet-toml refuses to
    /// serialize the absent `author`, so `to_comment` serializes present fields
    /// only.
    #[test]
    fn to_comment_round_trips_partial_meta() {
        let meta = NoteMeta {
            author: None,
            kind: Some("question".into()),
            ..Default::default()
        };
        let comment = to_comment(&meta, "body").expect("serializable");
        let parsed = parse_note(&comment).expect("round-trips");
        assert!(parsed.meta.author.is_none());
        assert_eq!(parsed.meta.kind.as_deref(), Some("question"));
        assert_eq!(parsed.body, "body");
    }

    #[test]
    fn to_comment_rejects_terminator_in_body() {
        assert!(to_comment(&NoteMeta::default(), "has --> inside").is_none());
    }

    #[test]
    fn to_comment_round_trips_resolved() {
        let meta = NoteMeta {
            id: Some("t1".into()),
            resolved: Some(true),
            ..Default::default()
        };
        let comment = to_comment(&meta, "done").expect("serializable");
        let parsed = parse_note(&comment).expect("round-trips");
        assert_eq!(parsed.meta.id.as_deref(), Some("t1"));
        assert_eq!(parsed.meta.resolved, Some(true));
        assert!(render_aside(&parsed.meta, "<p>x</p>").contains("data-resolved=\"true\""));
    }

    #[test]
    fn to_comment_round_trips_quote_and_aside_emits_it() {
        let meta = NoteMeta {
            id: Some("t1".into()),
            quote: Some("only after we reproduced it ourselves".into()),
            ..Default::default()
        };
        let comment = to_comment(&meta, "body").expect("serializable");
        let parsed = parse_note(&comment).expect("round-trips");
        assert_eq!(
            parsed.meta.quote.as_deref(),
            Some("only after we reproduced it ourselves")
        );
        assert!(
            render_aside(&parsed.meta, "<p>x</p>")
                .contains("data-quote=\"only after we reproduced it ourselves\"")
        );
    }

    #[test]
    fn wrap_and_strip_marks_round_trip() {
        let wrapped = wrap_mark("hello **world**", None);
        assert_eq!(wrapped, "<dodeca-mark>hello **world**</dodeca-mark>");
        assert_eq!(strip_marks(&wrapped), "hello **world**");
        // With an id (attributes on the open tag) it still strips cleanly.
        let with_id = wrap_mark("span", Some("abc123"));
        assert_eq!(
            with_id,
            "<dodeca-mark data-note-id=\"abc123\">span</dodeca-mark>"
        );
        assert_eq!(strip_marks(&with_id), "span");
        // Stripping leaves non-mark HTML untouched.
        assert_eq!(strip_marks("<p>x</p>"), "<p>x</p>");
    }

    #[test]
    fn aside_carries_kind_and_author() {
        let meta = NoteMeta {
            author: Some("amos".into()),
            kind: Some("question".into()),
            ..Default::default()
        };
        let html = render_aside(&meta, "<p>hi</p>");
        assert_eq!(
            html,
            "<aside class=\"dodeca-note\" data-kind=\"question\" data-author=\"amos\"><p>hi</p></aside>"
        );
    }
}
