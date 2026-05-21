+++
title = "Search"
weight = 70
+++

Every dodeca site has full-text search built in. There is nothing to enable and
nothing to configure — `ddc build` indexes your pages and the search box appears
in the site navigation.

## Using search

Click the search box in the navigation, or press `/` — or `Cmd-K` (`Ctrl-K` on
Windows and Linux) — from anywhere on the page. Results appear as you type:

- **Ranked** by relevance (BM25), best first.
- **Excerpts** show the surrounding text with your terms highlighted.
- **Deep links** jump straight to the matching section. On browsers that
  support [text fragments][], the matched text itself is scrolled to and
  highlighted.

Multiple words are combined with AND — a page must contain every word. The last
word matches by prefix, so results stay sensible mid-keystroke. Use the arrow
keys to move through results and `Enter` to open the selected one; `Esc` closes
the dropdown.

## What gets indexed

dodeca indexes the visible text of every page — headings, paragraphs, lists,
code. Site chrome (navigation, headers, footers, scripts and styles) is
excluded, so results point at real content. Each page's title comes from its
`<title>`, falling back to its first heading.

## Fully static

Search runs entirely in the browser. The index is a set of compact files
written under `/search/` next to your pages, and the query engine is a small
WebAssembly module. There is no search server and no runtime: a dodeca site
with search deploys to GitHub Pages — or any static host — unchanged.

The browser fetches the index lazily — a small manifest up front, then only the
shards a particular query needs. A follow-up query that reuses those shards
touches the network not at all.

## How it works

At build time the `cell-search` indexer receives the rendered HTML of every
page, extracts its text and heading structure, and builds a *sharded inverted
index*: terms grouped by first letter, each carrying the documents and
positions where it occurs. In the browser, the WebAssembly core decodes the
shards it needs, ranks matches with BM25, and renders the result list.

Indexing and querying share a single Rust crate, so a word is tokenized and
stemmed identically whether it is being indexed or searched for — what you
write is what you can find.

[text fragments]: https://developer.mozilla.org/en-US/docs/Web/URI/Fragment/Text_fragments
