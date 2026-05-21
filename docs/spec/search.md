# Full-Text Search Specification

## Introduction

dodeca ships a from-scratch, pagefind-inspired full-text search: a build-time
indexer (`cell-search`), a shared on-disk format and query engine
(`dodeca-search-format`), and a browser-side WebAssembly query core
(`dodeca-search-wasm`) driven by a small UI script.

This specification defines how text is analyzed, how the index is laid out,
how queries are ranked, how results are rendered, and how the assets are
served. It is the source of truth for implementation correctness via tracey
coverage.

---

# Text Analysis

Analysis turns text into the tokens that are both indexed and queried. The
indexer and the query engine MUST analyze text through the same code path, so
that a word indexed from a page is found by the same word typed as a query.

> s[analyze.tokenize]
> Text MUST be split into word tokens on Unicode word boundaries (UAX #29).
> Punctuation and whitespace between words are not tokens.

> s[analyze.lowercase]
> Each token MUST be lowercased before stemming, so that matching is
> case-insensitive.

> s[analyze.stem]
> Each lowercased token MUST be reduced to a stem using the Snowball English
> stemmer. The stem is the key stored in the inverted index.

> s[analyze.consistent]
> The indexer and the query engine MUST use the identical analysis function.
> There is no second tokenizer.

> s[analyze.display-form]
> Analysis MUST also retain each token's original surface form. The surface
> forms are what an excerpt displays; the stems are what the index keys on.

---

# Index Format

The index is a set of postcard-encoded files served under `/search/`.

> s[format.encoding]
> All index files (manifest, shards, fragments) MUST be encoded with
> facet-postcard. They are produced and consumed by the same dodeca version, so
> the schema is always in sync.

> s[format.manifest]
> A single manifest file MUST list every indexed document (URL, title, token
> length, fragment path) and every inverted-index shard.

> s[format.shard-prefix]
> Inverted-index terms MUST be partitioned into shards by their first ASCII
> alphanumeric character. Terms not starting with an ASCII alphanumeric go in a
> single catch-all shard.

> s[format.terms-sorted]
> Within a shard, terms MUST be stored sorted, so the reader can binary-search
> an exact term and range-scan a prefix.

> s[format.postings]
> Each term MUST carry its postings sorted by document id; each posting MUST
> carry the term's positions within that document in ascending order.

> s[format.fragment]
> Each document MUST have a fragment file holding its display token list (in
> original surface form) and its heading anchors, used to render excerpts.

---

# Indexing

The indexer receives the rendered HTML of every page.

> s[index.skip-chrome]
> When extracting page text, the indexer MUST skip the subtrees of `script`,
> `style`, `nav`, `header`, `footer`, `aside`, `template` and `noscript`
> elements — site chrome is not page content.

> s[index.content-root]
> The indexer MUST index the first `<main>` element if the page has one,
> otherwise the whole `<body>`.

> s[index.title]
> A document's title MUST be taken from the `<title>` element, falling back to
> the first `<h1>`, falling back to the page URL.

> s[index.anchors]
> A heading element (`h1`–`h6`) carrying an `id` MUST become an anchor recorded
> at the word position where the heading begins, enabling deep links.

> s[index.doc-length]
> Each document's length MUST be its token count, recorded for BM25 length
> normalization.

---

# Query

> s[query.and]
> A query of multiple words MUST match a document only if every word matches
> it (AND semantics).

> s[query.prefix]
> The final query word MUST match any indexed term that begins with its stem,
> giving as-you-type behaviour. All earlier words match their exact stem.

> s[query.bm25]
> Matching documents MUST be ranked by summed BM25 score, best first. When a
> prefix word matches several indexed terms, only the strongest counts toward
> the score.

> s[query.shard-selection]
> A query MUST fetch only the shards its words' prefixes resolve to — not the
> whole index.

---

# Result Rendering

> s[render.excerpt]
> An excerpt MUST be the fixed-width window of a document's display tokens that
> covers the most matched positions.

> s[render.mark]
> In the excerpt, matched words MUST be wrapped in `<mark>`; all excerpt text
> MUST be HTML-escaped.

> s[render.deeplink]
> A result's link MUST point to the nearest heading anchor at or before the
> first matched word, so the user lands in the relevant section.

> s[render.text-fragment]
> A result's link MUST also carry a [text-fragment][] directive
> (`:~:text=start[,end]`) spanning the matched words, so a browser that
> supports it scrolls to and highlights the match itself. The heading anchor
> remains the fallback for browsers that do not.
>
> [text-fragment]: https://developer.mozilla.org/en-US/docs/Web/URI/Fragment/Text_fragments

---

# Serving

> s[serve.index-paths]
> The index files MUST be served at fixed paths: the manifest at
> `/search/meta`, shards at `/search/index/<prefix>`, fragments at
> `/search/fragment/<id>`.

> s[serve.runtime]
> The search runtime assets — the WASM query core, its loader, the UI script
> and the stylesheet — MUST be served from a content-versioned directory under
> `/search/`, so they can be cached immutably and a new `ddc` serves them at
> fresh URLs.

> s[serve.inject]
> Every rendered page MUST have the search stylesheet and UI script injected
> into its `<head>`, so the theme's search slot becomes functional.

> s[serve.both-modes]
> The index MUST be available identically from `ddc build` output and from
> `ddc serve`, so dev and production search behave the same.

---

# Versioning

> s[version.stamp]
> The manifest MUST carry the format version it was written with.

> s[version.reject]
> The reader MUST refuse a manifest whose format version it does not
> recognize, rather than misinterpreting it.
