// Dodeca inline-note annotation overlay.
//
// Server-rendered structure (dev only, via marq's render_notes):
//   <dodeca-mark data-note-id="ID">…the highlighted span…</dodeca-mark>
//   <aside class="dodeca-note" data-note-id data-kind data-author data-created>
//     …rendered markdown body…
//   </aside>
//
// This bundle turns that into an interactive review-comment layer:
//   - highlights are the affordance; click one to open its note card,
//   - a note index (top-right) lists every note and scrolls to it,
//   - gutter markers by the scrollbar show where notes are,
//   - selecting text opens a popup to author a new note (⌘↵ to save).
//
// It's a standalone bundle with its own vox connection — independent of the
// WASM devtools and the Monaco editor.

import { connect, voxServiceMetadata, Driver } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import { DevtoolsServiceClient, type AnnotateResult } from "./devtools.generated";
import { BrowserServiceDispatcher } from "./browser.generated";

// ── styling ───────────────────────────────────────────────────────────────
// Injected at runtime so the bundle is a single self-contained module. In
// production there is no overlay (and marq strips the marks), so none of this
// applies; here it powers the dev annotation layer.
const KIND_COLORS: Record<string, string> = {
  note: "#89b4fa",
  question: "#f9e2af",
  todo: "#f38ba8",
};
// Square, business-like (GitHub review comments). No border-radius anywhere.
const STYLES = `
:root { --dn-note: #89b4fa; --dn-question: #f9e2af; --dn-todo: #f38ba8; }

/* The highlighted span: keep the (light) text readable; mark it with an accent
   underline + faint tint rather than a solid fill. */
dodeca-mark {
  background: color-mix(in srgb, var(--dn-note) 12%, transparent);
  border-bottom: 2px solid var(--dn-note);
  cursor: pointer;
  transition: background 0.12s ease;
}
dodeca-mark:hover, dodeca-mark.dn-active {
  background: color-mix(in srgb, var(--dn-note) 26%, transparent);
}
/* Resolved threads: highlight drops to plain text unless we're showing resolved. */
dodeca-mark.dn-resolved { background: none; border-bottom: none; cursor: auto; }
html.dn-show-resolved dodeca-mark.dn-resolved {
  background: color-mix(in srgb, var(--dn-note) 7%, transparent);
  border-bottom: 1px dashed var(--dn-note); cursor: pointer;
}

/* Server-rendered note bodies are data only; the overlay renders cards. */
aside.dodeca-note { display: none !important; }

/* ── note index (top-right) ── */
.dn-index {
  position: fixed; top: 12px; right: 12px; z-index: 2147483640;
  font: 13px/1.4 system-ui, sans-serif; color: #cdd6f4;
}
.dn-index-toggle {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 6px 12px; border: none; border-radius: 2px; cursor: pointer;
  background: #1e1e2e; color: #cdd6f4; font: 600 12px system-ui, sans-serif;
  box-shadow: 0 3px 10px rgba(0,0,0,0.3);
}
.dn-index-toggle:hover { filter: brightness(1.12); }
/* Absolute so opening the panel doesn't resize/shift the toggle. */
.dn-index-panel {
  position: absolute; right: 0; top: calc(100% + 4px);
  width: 300px; max-height: 60vh; overflow-y: auto; border-radius: 2px;
  background: #1e1e2e; box-shadow: 0 10px 40px rgba(0,0,0,0.4);
}
.dn-index-panel[hidden] { display: none; }
.dn-index-head {
  padding: 8px 10px; font-size: 11px; border-bottom: 1px solid #313244;
  display: flex; align-items: center; justify-content: space-between; gap: 8px;
}
.dn-index-head .dn-head-count { opacity: 0.6; text-transform: uppercase; letter-spacing: 0.05em; }
.dn-index-head label { display: inline-flex; align-items: center; gap: 4px; cursor: pointer; opacity: 0.85; }
.dn-index-item {
  display: block; width: 100%; text-align: left; cursor: pointer;
  background: transparent; border: none; color: inherit; font: inherit;
  padding: 8px 10px; border-left: 3px solid var(--dn-note); border-bottom: 1px solid #26273a;
}
.dn-index-item:hover { background: #313244; }
.dn-index-item.dn-resolved { display: none; opacity: 0.55; }
.dn-index-item.dn-resolved .dn-snip { text-decoration: line-through; }
html.dn-show-resolved .dn-index-item.dn-resolved { display: block; }
.dn-index-item .dn-meta { font-size: 11px; opacity: 0.6; display: flex; gap: 6px; }
.dn-index-item .dn-snip { display: block; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 1px; }
.dn-empty { padding: 10px; opacity: 0.5; font-size: 12px; }

/* ── gutter markers (right edge) ── */
.dn-gutter { position: fixed; top: 0; right: 0; width: 10px; height: 100vh; z-index: 2147483639; pointer-events: none; }
.dn-gutter-mark {
  position: absolute; right: 2px; width: 6px; height: 6px; border-radius: 2px;
  background: var(--dn-note); cursor: pointer; pointer-events: auto;
  box-shadow: 0 0 0 1px rgba(0,0,0,0.15);
}
.dn-gutter-mark:hover { transform: scale(1.6); }
.dn-gutter-mark.dn-resolved { display: none; }
html.dn-show-resolved .dn-gutter-mark.dn-resolved { display: block; opacity: 0.5; }

/* ── note card (anchored popover) ── */
.dn-card {
  position: absolute; z-index: 2147483641; width: 340px;
  display: flex; flex-direction: column; max-height: 70vh; border-radius: 2px;
  background: #1e1e2e; color: #cdd6f4;
  box-shadow: 0 10px 40px rgba(0,0,0,0.45);
  border-left: 3px solid var(--dn-note);
  font: 13px/1.5 system-ui, sans-serif;
}
/* Comments scroll; the footer (resolve + reply) stays put for long threads. */
.dn-card-scroll { overflow-y: auto; flex: 1 1 auto; min-height: 0; }
.dn-card-comment { padding: 10px 12px; border-bottom: 1px solid #313244; }
.dn-card-byline { display: flex; align-items: baseline; gap: 6px; margin-bottom: 4px; font-size: 11px; }
.dn-card-author { font-weight: 700; }
.dn-card-kind { text-transform: uppercase; letter-spacing: 0.05em; padding: 0 5px; background: #313244; }
.dn-card-date { opacity: 0.5; margin-left: auto; }
.dn-card-body > :first-child { margin-top: 0; }
.dn-card-body > :last-child { margin-bottom: 0; }
.dn-card-body p { margin: 0.3em 0; }
.dn-reply { padding: 8px 12px; background: #181825; flex: 0 0 auto; }
.dn-reply textarea {
  width: 100%; box-sizing: border-box; resize: vertical; min-height: 44px; border-radius: 2px;
  background: #11111b; color: #cdd6f4; border: 1px solid #45475a; padding: 6px; font: inherit;
}
.dn-reply-row { display: flex; gap: 6px; align-items: center; margin-top: 6px; }
.dn-reply-author { flex: 1; border-radius: 2px; background: #313244; color: #cdd6f4; border: 1px solid #45475a; padding: 3px 5px; font: inherit; }
.dn-reply-status { min-height: 1em; margin-top: 4px; opacity: 0.7; font-size: 11px; }
.dn-btn-resolve { background: transparent; color: #cdd6f4; opacity: 0.7; border: 1px solid #45475a; }
.dn-btn-resolve:hover { opacity: 1; background: #313244; }

/* ── create popup ── */
.dn-create {
  position: absolute; z-index: 2147483646; width: 340px; padding: 8px; border-radius: 2px;
  background: #1e1e2e; color: #cdd6f4;
  box-shadow: 0 8px 30px rgba(0,0,0,0.35); font: 13px/1.4 system-ui, sans-serif;
}
.dn-create[hidden] { display: none; }
.dn-create .dn-row { display: flex; gap: 6px; align-items: center; margin-bottom: 6px; }
.dn-create input {
  background: #313244; color: #cdd6f4; border: 1px solid #45475a; border-radius: 2px; padding: 3px 5px; font: inherit;
}
.dn-create .dn-author { flex: 1; }
.dn-create .dn-quote { flex: 1; opacity: 0.55; font-style: italic; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dn-create textarea {
  width: 100%; box-sizing: border-box; resize: vertical; min-height: 56px; border-radius: 2px;
  background: #11111b; color: #cdd6f4; border: 1px solid #45475a; padding: 6px; font: inherit;
}
.dn-create .dn-status { min-height: 1.2em; margin-top: 4px; opacity: 0.7; font-size: 12px; }

/* segmented kind picker */
.dn-seg { display: flex; gap: 0; overflow: hidden; border: 1px solid #45475a; border-radius: 2px; }
.dn-seg-btn {
  flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: 5px;
  padding: 5px 8px; cursor: pointer; border: none; border-right: 1px solid #45475a;
  background: #313244; color: #cdd6f4; font: 600 12px system-ui, sans-serif;
}
.dn-seg-btn:last-child { border-right: none; }
.dn-seg-btn:hover { background: #3b3d52; }
.dn-seg-btn.dn-on { color: #11111b; }
.dn-seg-btn.dn-on[data-kind="note"] { background: var(--dn-note); }
.dn-seg-btn.dn-on[data-kind="question"] { background: var(--dn-question); }
.dn-seg-btn.dn-on[data-kind="todo"] { background: var(--dn-todo); }

/* keycap hints + action buttons */
kbd.dn-kbd {
  font: 600 10px ui-monospace, monospace; line-height: 1;
  padding: 2px 4px; border-radius: 2px; background: rgba(0,0,0,0.25);
  border: 1px solid rgba(255,255,255,0.12); opacity: 0.85;
}
.dn-seg-btn.dn-on kbd.dn-kbd { background: rgba(0,0,0,0.18); border-color: rgba(0,0,0,0.2); }
.dn-actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; margin-top: 6px; }
.dn-btn {
  display: inline-flex; align-items: center; gap: 6px; cursor: pointer;
  border: none; border-radius: 2px; padding: 6px 12px; font: 600 12px system-ui, sans-serif;
}
.dn-btn-ghost { background: transparent; color: #cdd6f4; opacity: 0.7; }
.dn-btn-ghost:hover { opacity: 1; background: #313244; }
.dn-btn-save { background: #89b4fa; color: #11111b; }
.dn-btn-save:hover { filter: brightness(1.1); }
`;

function injectStyles(): void {
  const style = document.createElement("style");
  style.dataset.dodecaAnnotate = "";
  style.textContent = STYLES;
  document.head.appendChild(style);
}

// ── connection (lazy; the UI never blocks on it) ────────────────────────────
function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
}
let clientPromise: Promise<DevtoolsServiceClient> | null = null;
function client(): Promise<DevtoolsServiceClient> {
  if (!clientPromise) {
    clientPromise = (async () => {
      const connection = await connect(wsConnector(wsUrl()));
      const lane = await connection.openRawLane({
        metadata: voxServiceMetadata("DevtoolsService"),
      });
      // The host may push BrowserService events on this lane; the overlay
      // doesn't need them (it reloads after writes), so the dispatcher is a
      // no-op. The Driver must still run to service the lane.
      void Driver.new(lane, new BrowserServiceDispatcher({ onEvent: async () => {} })).run();
      return new DevtoolsServiceClient(lane.caller());
    })();
  }
  return clientPromise;
}

// ── note model (read from server-rendered asides) ───────────────────────────
interface NoteComment {
  author: string;
  kind: string;
  created: string; // RFC3339, possibly ""
  bodyHTML: string;
}
interface Note {
  id: string;
  mark: HTMLElement | null;
  comments: NoteComment[];
  resolved: boolean;
}

function collectNotes(): Note[] {
  const byId = new Map<string, Note>();
  const note = (id: string): Note => {
    let n = byId.get(id);
    if (!n) {
      n = {
        id,
        mark: document.querySelector<HTMLElement>(`dodeca-mark[data-note-id="${CSS.escape(id)}"]`),
        comments: [],
        resolved: false,
      };
      byId.set(id, n);
    }
    return n;
  };
  for (const aside of document.querySelectorAll<HTMLElement>("aside.dodeca-note")) {
    const id = aside.dataset.noteId;
    if (!id) continue;
    const n = note(id);
    if (aside.dataset.resolved === "true") n.resolved = true;
    n.comments.push({
      author: aside.dataset.author ?? "",
      kind: aside.dataset.kind ?? "note",
      created: aside.dataset.created ?? "",
      bodyHTML: aside.innerHTML,
    });
  }
  // Stable order: by the mark's document position when present, else insertion.
  return [...byId.values()].sort((a, b) => markTop(a) - markTop(b));
}

function markTop(n: Note): number {
  if (!n.mark) return Number.MAX_SAFE_INTEGER;
  const r = n.mark.getBoundingClientRect();
  return r.top + window.scrollY;
}

function fmtDate(rfc: string): string {
  if (!rfc) return "";
  const d = new Date(rfc);
  if (isNaN(d.getTime())) return "";
  const now = new Date();
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: sameYear ? undefined : "numeric" });
}

function kindColor(kind: string): string {
  return KIND_COLORS[kind] ?? KIND_COLORS.note;
}

// ── main ────────────────────────────────────────────────────────────────────
const AUTHOR_KEY = "dodeca-note-author";
const SHOW_RESOLVED_KEY = "dodeca-show-resolved";

function showingResolved(): boolean {
  return document.documentElement.classList.contains("dn-show-resolved");
}

function main(): void {
  injectStyles();
  document.documentElement.classList.toggle(
    "dn-show-resolved",
    localStorage.getItem(SHOW_RESOLVED_KEY) === "1",
  );

  const notes = collectNotes();
  const layer = document.createElement("div");
  layer.className = "dodeca-annotate-ui";
  document.body.appendChild(layer);

  // One card at a time. It opens on hover (preview) and is "pinned" on click so
  // it stays while you interact with it; an unpinned card closes on mouse-out.
  let openCard: HTMLElement | null = null;
  let pinned = false;
  let closeTimer: number | undefined;
  const cancelClose = () => {
    if (closeTimer) clearTimeout(closeTimer);
    closeTimer = undefined;
  };
  const closeCard = () => {
    cancelClose();
    openCard?.remove();
    openCard = null;
    pinned = false;
    document.querySelectorAll("dodeca-mark.dn-active").forEach((m) => m.classList.remove("dn-active"));
  };
  const closeSoon = () => {
    cancelClose();
    closeTimer = window.setTimeout(() => {
      if (!pinned) closeCard();
    }, 160);
  };

  // A reply box at the foot of a card: posts a comment onto the same thread.
  const buildReplyForm = (note: Note): HTMLElement => {
    const wrap = document.createElement("div");
    wrap.className = "dn-reply";
    const ta = document.createElement("textarea");
    ta.placeholder = "Reply…";
    const row = document.createElement("div");
    row.className = "dn-reply-row";
    const author = document.createElement("input");
    author.className = "dn-reply-author";
    author.placeholder = "your name";
    author.value = localStorage.getItem(AUTHOR_KEY) ?? "";
    const resolve = document.createElement("button");
    resolve.className = "dn-btn dn-btn-resolve";
    resolve.textContent = note.resolved ? "Reopen" : "Resolve";
    const send = document.createElement("button");
    send.className = "dn-btn dn-btn-save";
    send.innerHTML = `Reply <kbd class="dn-kbd">⌘↵</kbd>`;
    const status = document.createElement("div");
    status.className = "dn-reply-status";
    row.append(author, resolve, send);
    wrap.append(ta, row, status);

    // Interacting with the reply box pins the card so it doesn't close.
    ta.addEventListener("focus", () => {
      pinned = true;
      cancelClose();
    });

    resolve.addEventListener("click", async () => {
      pinned = true;
      status.textContent = note.resolved ? "reopening…" : "resolving…";
      try {
        const res: AnnotateResult = await (await client()).setNoteResolved(
          location.pathname,
          note.id,
          !note.resolved,
        );
        if (res.tag === "Ok") {
          status.textContent = "saved — reloading…";
          setTimeout(() => location.reload(), 250);
        } else {
          status.textContent = res.tag === "NotFound" ? "thread not found" : `error: ${res.message}`;
        }
      } catch (err) {
        status.textContent = `failed: ${String(err)}`;
      }
    });

    const submit = async () => {
      const body = ta.value.trim();
      if (!body) return;
      const a = author.value.trim();
      localStorage.setItem(AUTHOR_KEY, a);
      status.textContent = "saving…";
      try {
        const res: AnnotateResult = await (await client()).annotate({
          route: location.pathname,
          sid: "",
          selected_text: "",
          body,
          author: a || null,
          kind: null,
          reply_to: note.id,
        });
        if (res.tag === "Ok") {
          status.textContent = "saved — reloading…";
          setTimeout(() => location.reload(), 250);
        } else if (res.tag === "NotFound") {
          status.textContent = "thread not found";
        } else {
          status.textContent = `error: ${res.message}`;
        }
      } catch (err) {
        status.textContent = `failed: ${String(err)}`;
      }
    };
    send.addEventListener("click", () => void submit());
    ta.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void submit();
      }
    });
    return wrap;
  };

  // Render a note's thread as an anchored card next to its mark.
  let openNoteRef: Note | null = null;
  const openNote = (note: Note, pin: boolean) => {
    // Resolved threads don't open unless we're showing resolved.
    if (note.resolved && !showingResolved()) return;
    if (note !== openNoteRef) closeCard();
    openNoteRef = note;
    pinned = pinned || pin;
    if (!note.mark) return;
    if (openCard) {
      // Already showing this note; just (maybe) pin it.
      return;
    }
    note.mark.classList.add("dn-active");
    const card = document.createElement("div");
    card.className = "dn-card";
    card.style.borderLeftColor = kindColor(note.comments[0]?.kind ?? "note");
    const scroll = document.createElement("div");
    scroll.className = "dn-card-scroll";
    for (const c of note.comments) {
      const el = document.createElement("div");
      el.className = "dn-card-comment";
      const byline = document.createElement("div");
      byline.className = "dn-card-byline";
      const dateEl = document.createElement("span");
      dateEl.className = "dn-card-date";
      dateEl.textContent = fmtDate(c.created);
      if (c.created) dateEl.title = new Date(c.created).toLocaleString();
      const author = document.createElement("span");
      author.className = "dn-card-author";
      author.textContent = c.author || "anon";
      const kindEl = document.createElement("span");
      kindEl.className = "dn-card-kind";
      kindEl.style.color = kindColor(c.kind);
      kindEl.textContent = c.kind;
      byline.append(author, kindEl, dateEl);
      const body = document.createElement("div");
      body.className = "dn-card-body";
      body.innerHTML = c.bodyHTML;
      el.append(byline, body);
      scroll.appendChild(el);
    }
    card.appendChild(scroll);
    card.appendChild(buildReplyForm(note));
    card.addEventListener("mouseenter", cancelClose);
    card.addEventListener("mouseleave", closeSoon);
    // Clicking inside the card pins it.
    card.addEventListener("click", () => {
      pinned = true;
    });
    layer.appendChild(card);
    // Anchor below the mark, clamped into the viewport horizontally.
    const r = note.mark.getBoundingClientRect();
    card.style.top = `${window.scrollY + r.bottom + 6}px`;
    card.style.left = `${Math.max(8, window.scrollX + Math.min(r.left, window.innerWidth - 348))}px`;
    openCard = card;
  };

  const scrollToNote = (note: Note) => {
    if (!note.mark) return;
    note.mark.scrollIntoView({ behavior: "smooth", block: "center" });
    setTimeout(() => openNote(note, true), 320);
  };

  // Hover previews a note; click pins it.
  for (const note of notes) {
    if (note.resolved) note.mark?.classList.add("dn-resolved");
    note.mark?.addEventListener("mouseenter", () => openNote(note, false));
    note.mark?.addEventListener("mouseleave", closeSoon);
    note.mark?.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      openNote(note, true);
    });
  }
  // Click-away closes a pinned card.
  document.addEventListener("click", (e) => {
    const t = e.target as Element | null;
    if (openCard && !openCard.contains(t) && !t?.closest?.("dodeca-mark")) closeCard();
  });

  buildIndex(layer, notes, scrollToNote);
  buildGutter(layer, notes, scrollToNote);
  installCreateUI(layer);
  console.log(`[dodeca-annotate] ready — ${notes.length} note(s)`);
}

// ── note index (top-right) ───────────────────────────────────────────────────
function buildIndex(layer: HTMLElement, notes: Note[], onPick: (n: Note) => void): void {
  const wrap = document.createElement("div");
  wrap.className = "dn-index";
  const open = notes.filter((n) => !n.resolved);
  const resolvedCount = notes.length - open.length;
  const toggle = document.createElement("button");
  toggle.className = "dn-index-toggle";
  toggle.innerHTML = `📝 <span>${open.length}</span>`;
  const panel = document.createElement("div");
  panel.className = "dn-index-panel";
  panel.hidden = true;

  const head = document.createElement("div");
  head.className = "dn-index-head";
  const count = document.createElement("span");
  count.className = "dn-head-count";
  count.textContent = `${open.length} note${open.length === 1 ? "" : "s"}`;
  head.appendChild(count);
  if (resolvedCount > 0) {
    const label = document.createElement("label");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = showingResolved();
    cb.addEventListener("change", () => {
      document.documentElement.classList.toggle("dn-show-resolved", cb.checked);
      localStorage.setItem(SHOW_RESOLVED_KEY, cb.checked ? "1" : "0");
    });
    label.append(cb, document.createTextNode(`show ${resolvedCount} resolved`));
    head.appendChild(label);
  }
  panel.appendChild(head);

  if (notes.length === 0) {
    const empty = document.createElement("div");
    empty.className = "dn-empty";
    empty.textContent = "No notes on this page yet — select text to add one.";
    panel.appendChild(empty);
  }
  for (const note of notes) {
    const first = note.comments[0];
    const item = document.createElement("button");
    item.className = note.resolved ? "dn-index-item dn-resolved" : "dn-index-item";
    item.style.borderLeftColor = kindColor(first?.kind ?? "note");
    const snippet = (note.mark?.textContent ?? first?.bodyHTML ?? "").replace(/<[^>]*>/g, "").trim();
    item.innerHTML =
      `<span class="dn-meta"><b>${first?.author || "anon"}</b><span>${first?.kind ?? ""}</span>` +
      `<span style="margin-left:auto;opacity:.7">${fmtDate(first?.created ?? "")}</span></span>` +
      `<span class="dn-snip">${snippet || "(note)"}</span>`;
    item.addEventListener("click", () => {
      panel.hidden = true;
      onPick(note);
    });
    panel.appendChild(item);
  }

  toggle.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
  });
  wrap.append(toggle, panel);
  layer.appendChild(wrap);
}

// ── gutter markers (right edge) ──────────────────────────────────────────────
function buildGutter(layer: HTMLElement, notes: Note[], onPick: (n: Note) => void): void {
  const gutter = document.createElement("div");
  gutter.className = "dn-gutter";
  layer.appendChild(gutter);

  const place = () => {
    gutter.innerHTML = "";
    const docH = Math.max(document.documentElement.scrollHeight, 1);
    for (const note of notes) {
      if (!note.mark) continue;
      const top = markTop(note);
      const mark = document.createElement("div");
      mark.className = note.resolved ? "dn-gutter-mark dn-resolved" : "dn-gutter-mark";
      mark.style.top = `${(top / docH) * 100}vh`;
      mark.style.background = kindColor(note.comments[0]?.kind ?? "note");
      mark.title = `${note.comments[0]?.author || "anon"}: ${(note.mark.textContent ?? "").slice(0, 40)}`;
      mark.addEventListener("click", () => onPick(note));
      gutter.appendChild(mark);
    }
  };
  place();
  window.addEventListener("resize", place);
}

// ── create popup (select text → author a note) ───────────────────────────────
interface Target {
  sid: string;
  text: string;
}
function targetForSelection(sel: Selection): Target | null {
  if (sel.rangeCount === 0 || sel.isCollapsed) return null;
  const text = sel.toString().trim();
  if (!text) return null;
  const node = sel.getRangeAt(0).commonAncestorContainer;
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  if (!el || el.closest(".dodeca-annotate-ui")) return null;
  const sidEl = el.closest("[data-sid]");
  const sid = sidEl?.getAttribute("data-sid");
  return sid ? { sid, text } : null;
}

// Kinds with their Alt-key shortcut. `code` matches the physical key (reliable
// across layouts / the Mac Option-composes-characters problem).
const KINDS: { kind: string; code: string; label: string; hint: string }[] = [
  { kind: "note", code: "KeyN", label: "note", hint: "⌥N" },
  { kind: "question", code: "KeyQ", label: "question", hint: "⌥Q" },
  { kind: "todo", code: "KeyT", label: "todo", hint: "⌥T" },
];

function installCreateUI(layer: HTMLElement): void {
  const ui = document.createElement("div");
  ui.className = "dn-create";
  ui.hidden = true;
  const segs = KINDS.map(
    (k) =>
      `<button class="dn-seg-btn" data-kind="${k.kind}">${k.label}` +
      `<kbd class="dn-kbd">${k.hint}</kbd></button>`,
  ).join("");
  ui.innerHTML = `
    <div class="dn-seg">${segs}</div>
    <div class="dn-row" style="margin-top:6px">
      <input class="dn-author" type="text" placeholder="your name" />
      <span class="dn-quote"></span>
    </div>
    <textarea class="dn-body" placeholder="Write a note…"></textarea>
    <div class="dn-actions">
      <button class="dn-btn dn-btn-ghost dn-cancel">Cancel <kbd class="dn-kbd">Esc</kbd></button>
      <button class="dn-btn dn-btn-save dn-save">Save <kbd class="dn-kbd">⌘↵</kbd></button>
    </div>
    <div class="dn-status"></div>
  `;
  layer.appendChild(ui);
  const authorEl = ui.querySelector(".dn-author") as HTMLInputElement;
  const quoteEl = ui.querySelector(".dn-quote") as HTMLElement;
  const bodyEl = ui.querySelector(".dn-body") as HTMLTextAreaElement;
  const statusEl = ui.querySelector(".dn-status") as HTMLElement;
  const segBtns = [...ui.querySelectorAll<HTMLButtonElement>(".dn-seg-btn")];
  authorEl.value = localStorage.getItem(AUTHOR_KEY) ?? "";

  let kind = "note";
  const setKind = (k: string) => {
    kind = k;
    for (const b of segBtns) b.classList.toggle("dn-on", b.dataset.kind === k);
  };
  setKind("note");
  for (const b of segBtns) b.addEventListener("click", () => setKind(b.dataset.kind!));

  let pending: Target | null = null;
  const hide = () => {
    ui.hidden = true;
    pending = null;
  };

  document.addEventListener("mouseup", (e) => {
    const t = e.target as Element | null;
    if (t?.closest?.(".dodeca-annotate-ui")) return;
    const sel = window.getSelection();
    const target = sel && targetForSelection(sel);
    if (!target) {
      if (pending) hide();
      return;
    }
    pending = target;
    quoteEl.textContent = target.text.length > 80 ? `${target.text.slice(0, 77)}…` : target.text;
    bodyEl.value = "";
    statusEl.textContent = "";
    setKind("note");
    ui.hidden = false;
    const r = sel!.getRangeAt(0).getBoundingClientRect();
    ui.style.top = `${window.scrollY + r.bottom + 8}px`;
    ui.style.left = `${Math.max(8, window.scrollX + Math.min(r.left, window.innerWidth - 360))}px`;
    bodyEl.focus();
  });

  const save = async () => {
    if (!pending) return;
    const body = bodyEl.value.trim();
    if (!body) return;
    const author = authorEl.value.trim();
    localStorage.setItem(AUTHOR_KEY, author);
    statusEl.textContent = "saving…";
    try {
      const res: AnnotateResult = await (await client()).annotate({
        route: location.pathname,
        sid: pending.sid,
        selected_text: pending.text,
        body,
        author: author || null,
        kind,
        reply_to: null,
      });
      switch (res.tag) {
        case "Ok":
          statusEl.textContent = "saved — reloading…";
          // The note now lives in source; reload to re-render + re-scan cleanly
          // (avoids fighting the WASM devtools' HMR over our injected DOM).
          setTimeout(() => location.reload(), 250);
          break;
        case "NotFound":
          statusEl.textContent = "couldn't map the selection back to source";
          break;
        case "Error":
          statusEl.textContent = `error: ${res.message}`;
          break;
      }
    } catch (err) {
      statusEl.textContent = `failed: ${String(err)}`;
    }
  };

  (ui.querySelector(".dn-save") as HTMLButtonElement).addEventListener("click", () => void save());
  (ui.querySelector(".dn-cancel") as HTMLButtonElement).addEventListener("click", hide);

  // Keyboard, handled at the popup level so the shortcuts work from any field:
  // Esc cancels, ⌘/Ctrl+↵ saves, ⌥+letter picks a kind (by physical key code).
  ui.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    } else if (e.altKey) {
      const match = KINDS.find((k) => k.code === e.code);
      if (match) {
        e.preventDefault();
        setKind(match.kind);
      }
    }
  });
}

main();
// Warm the connection so the first save is snappy; failures surface on save.
void client().catch((err) => console.error("[dodeca-annotate] connect failed:", err));
