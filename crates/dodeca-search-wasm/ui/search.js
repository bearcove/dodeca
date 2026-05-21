// dodeca search widget.
//
// Thin DOM/UX layer around the WASM query core (dodeca-search-wasm). The wasm
// module owns everything correctness-critical — fetching the postcard index,
// decoding it, BM25 ranking, excerpt rendering. This file owns only the input
// box, the results dropdown, debouncing and keyboard handling.
//
// It is served from a content-versioned directory (/search/asset/<v>/) next to
// the wasm module, and injected into every page as a `<script type="module">`.
// The wasm import is relative so it resolves within that same versioned
// directory automatically.

import initWasm, { load_index, search as runQuery } from "./dodeca_search_wasm.js";

// Container the docs template provides: `<div id="search">`.
const MOUNT_ID = "search";
// Idle gap before a keystroke turns into a query.
const DEBOUNCE_MS = 120;

function buildUi(mount) {
  mount.classList.add("ds-root");

  const input = document.createElement("input");
  input.type = "search";
  input.className = "ds-input";
  input.placeholder = "Search…";
  input.setAttribute("aria-label", "Search the documentation");
  input.autocomplete = "off";
  input.spellcheck = false;

  const results = document.createElement("div");
  results.className = "ds-results";
  results.hidden = true;

  mount.replaceChildren(input, results);
  return { mount, input, results };
}

async function boot() {
  const mount = document.getElementById(MOUNT_ID);
  if (!mount) return;

  const ui = buildUi(mount);

  try {
    await initWasm();
    await load_index("/search/meta");
  } catch (e) {
    console.error("[dodeca-search] index unavailable:", e);
    ui.input.placeholder = "Search unavailable";
    ui.input.disabled = true;
    return;
  }

  wireEvents(ui);
}

function wireEvents(ui) {
  const { mount, input, results } = ui;
  // Index of the keyboard-highlighted result, or -1 for none.
  let selected = -1;
  // Monotonic token so a slow query can't overwrite a newer one's results.
  let queryToken = 0;
  let debounce = 0;

  function close() {
    results.hidden = true;
    results.replaceChildren();
    selected = -1;
  }

  function highlight(next) {
    const items = results.querySelectorAll(".ds-result");
    if (items.length === 0) return;
    selected = (next + items.length) % items.length;
    items.forEach((el, i) => {
      el.classList.toggle("ds-selected", i === selected);
      if (i === selected) el.scrollIntoView({ block: "nearest" });
    });
  }

  function paint(hits) {
    selected = -1;
    if (hits.length === 0) {
      results.replaceChildren(emptyRow());
      results.hidden = false;
      return;
    }
    results.replaceChildren(...hits.map(resultRow));
    results.hidden = false;
  }

  async function query(text) {
    const token = ++queryToken;
    const trimmed = text.trim();
    if (trimmed === "") {
      close();
      return;
    }
    let hits;
    try {
      hits = JSON.parse(await runQuery(trimmed));
    } catch (e) {
      console.error("[dodeca-search] query failed:", e);
      return;
    }
    // A newer keystroke already superseded this query.
    if (token !== queryToken) return;
    paint(hits);
  }

  input.addEventListener("input", () => {
    clearTimeout(debounce);
    debounce = setTimeout(() => query(input.value), DEBOUNCE_MS);
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      highlight(selected + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      highlight(selected - 1);
    } else if (e.key === "Enter") {
      const items = results.querySelectorAll(".ds-result");
      const target = items[selected] || items[0];
      if (target) {
        e.preventDefault();
        window.location.href = target.href;
      }
    } else if (e.key === "Escape") {
      close();
      input.blur();
    }
  });

  // Click-away closes the dropdown.
  document.addEventListener("click", (e) => {
    if (!mount.contains(e.target)) close();
  });
  input.addEventListener("focus", () => {
    if (input.value.trim() !== "" && results.childElementCount > 0) {
      results.hidden = false;
    }
  });

  // Global shortcuts: Cmd/Ctrl-K and `/` jump to the search box.
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      input.focus();
      input.select();
    } else if (
      e.key === "/" &&
      e.target.tagName !== "INPUT" &&
      e.target.tagName !== "TEXTAREA" &&
      !e.target.isContentEditable
    ) {
      e.preventDefault();
      input.focus();
    }
  });
}

function resultRow(hit) {
  const a = document.createElement("a");
  a.className = "ds-result";
  a.href = hit.url;

  const title = document.createElement("div");
  title.className = "ds-result-title";
  title.textContent = hit.title;

  const excerpt = document.createElement("div");
  excerpt.className = "ds-result-excerpt";
  // `excerpt` is produced by the Rust renderer: plain text already HTML-escaped,
  // with only `<mark>` tags added around matched words. Safe to assign as HTML.
  excerpt.innerHTML = hit.excerpt;

  a.append(title, excerpt);
  return a;
}

function emptyRow() {
  const el = document.createElement("div");
  el.className = "ds-empty";
  el.textContent = "No results";
  return el;
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", boot);
} else {
  boot();
}
