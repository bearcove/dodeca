// Dodeca inline-note annotation overlay.
//
// Injected into dev pages. Select text on the page; a small popup appears where
// you type a note and press ⌘↵. The note is sent over a dedicated vox
// connection to DevtoolsService.annotate, which inserts a `<!-- note … -->`
// comment into the markdown source backing the selected element. Live reload
// then re-renders the page with the note shown as an <aside>.
//
// This bundle is intentionally standalone: its own vox connection, no shared
// state with the WASM devtools or the Monaco editor.

import { session, voxServiceMetadata } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import { DevtoolsServiceClient, type AnnotateResult } from "./devtools.generated";

// Styles are injected at runtime so the bundle is a single self-contained
// module (no separate stylesheet to <link>). Covers both the popup UI and the
// rendered note <aside> (dev-only, alongside the overlay).
const STYLES = `
.dodeca-annotate-ui {
  position: absolute;
  z-index: 2147483646;
  width: 340px;
  padding: 8px;
  border-radius: 8px;
  background: #1e1e2e;
  color: #cdd6f4;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.35);
  font: 13px/1.4 system-ui, sans-serif;
}
.dodeca-annotate-ui .da-head { display: flex; gap: 6px; align-items: center; margin-bottom: 6px; }
.dodeca-annotate-ui .da-kind {
  background: #313244; color: #cdd6f4; border: 1px solid #45475a;
  border-radius: 4px; padding: 2px 4px; font: inherit;
}
.dodeca-annotate-ui .da-quote {
  flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  opacity: 0.6; font-style: italic;
}
.dodeca-annotate-ui .da-body {
  width: 100%; box-sizing: border-box; resize: vertical;
  background: #11111b; color: #cdd6f4; border: 1px solid #45475a;
  border-radius: 4px; padding: 6px; font: inherit;
}
.dodeca-annotate-ui .da-status { min-height: 1.2em; margin-top: 4px; opacity: 0.7; font-size: 12px; }

aside.dodeca-note {
  margin: 1em 0; padding: 0.6em 0.9em;
  border-left: 3px solid #89b4fa;
  background: rgba(137, 180, 250, 0.08);
  border-radius: 0 6px 6px 0;
}
aside.dodeca-note > :first-child { margin-top: 0; }
aside.dodeca-note > :last-child { margin-bottom: 0; }
aside.dodeca-note[data-kind="question"] { border-left-color: #f9e2af; background: rgba(249, 226, 175, 0.08); }
aside.dodeca-note[data-kind="todo"] { border-left-color: #f38ba8; background: rgba(243, 139, 168, 0.08); }
aside.dodeca-note[data-author]::before {
  content: "note by " attr(data-author);
  display: block; font-size: 11px; text-transform: uppercase;
  letter-spacing: 0.04em; opacity: 0.5; margin-bottom: 0.3em;
}
`;

function injectStyles(): void {
  const style = document.createElement("style");
  style.dataset.dodecaAnnotate = "";
  style.textContent = STYLES;
  document.head.appendChild(style);
}

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
}

/** Open a dedicated DevtoolsService connection (Noop root + sub-connection). */
async function connect(): Promise<DevtoolsServiceClient> {
  const established = await session.initiator(wsConnector(wsUrl()), {
    metadata: voxServiceMetadata("Noop"),
  });
  const devtools = await established
    .handle()
    .openConnection(undefined, voxServiceMetadata("DevtoolsService"));
  return new DevtoolsServiceClient(devtools.caller());
}

interface Target {
  sid: string;
  text: string;
}

/** Resolve the current selection to the `data-sid` of its nearest mapped
 *  ancestor, plus the selected text. Returns null when there is no usable
 *  selection or it lands inside our own UI. */
function targetForSelection(sel: Selection): Target | null {
  if (sel.rangeCount === 0 || sel.isCollapsed) return null;
  const text = sel.toString().trim();
  if (!text) return null;

  const range = sel.getRangeAt(0);
  const node = range.commonAncestorContainer;
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  if (!el || el.closest(".dodeca-annotate-ui")) return null;

  const sidEl = el.closest("[data-sid]");
  const sid = sidEl?.getAttribute("data-sid");
  if (!sid) return null;
  return { sid, text };
}

function buildUi(): {
  root: HTMLElement;
  kind: HTMLSelectElement;
  quote: HTMLElement;
  body: HTMLTextAreaElement;
  status: HTMLElement;
} {
  const root = document.createElement("div");
  root.className = "dodeca-annotate-ui";
  root.style.display = "none";
  root.innerHTML = `
    <div class="da-head">
      <select class="da-kind" title="Note kind">
        <option value="note">note</option>
        <option value="question">question</option>
        <option value="todo">todo</option>
      </select>
      <span class="da-quote"></span>
    </div>
    <textarea class="da-body" rows="3"
      placeholder="Write a note…  (⌘↵ to save · Esc to cancel)"></textarea>
    <div class="da-status"></div>
  `;
  document.body.appendChild(root);
  return {
    root,
    kind: root.querySelector(".da-kind") as HTMLSelectElement,
    quote: root.querySelector(".da-quote") as HTMLElement,
    body: root.querySelector(".da-body") as HTMLTextAreaElement,
    status: root.querySelector(".da-status") as HTMLElement,
  };
}

function main(client: DevtoolsServiceClient): void {
  injectStyles();
  const ui = buildUi();
  let pending: Target | null = null;

  const hide = () => {
    ui.root.style.display = "none";
    pending = null;
  };

  const showAt = (rect: DOMRect, target: Target) => {
    pending = target;
    ui.quote.textContent = target.text.length > 80 ? `${target.text.slice(0, 77)}…` : target.text;
    ui.body.value = "";
    ui.status.textContent = "";
    ui.root.style.display = "block";
    // Anchor just below the end of the selection, clamped into the viewport.
    const top = window.scrollY + rect.bottom + 8;
    const left = window.scrollX + Math.min(rect.left, window.innerWidth - 360);
    ui.root.style.top = `${top}px`;
    ui.root.style.left = `${Math.max(8, left)}px`;
    ui.body.focus();
  };

  // Open the popup when a selection is finished outside our own UI.
  document.addEventListener("mouseup", (e) => {
    const t = e.target as Element | null;
    if (t?.closest?.(".dodeca-annotate-ui")) return;
    const sel = window.getSelection();
    if (!sel) return;
    const target = targetForSelection(sel);
    if (!target) {
      if (pending) hide();
      return;
    }
    showAt(sel.getRangeAt(0).getBoundingClientRect(), target);
  });

  ui.body.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
      return;
    }
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
    }
  });

  const save = async (): Promise<void> => {
    if (!pending) return;
    const body = ui.body.value.trim();
    if (!body) return;
    ui.status.textContent = "saving…";
    try {
      const res: AnnotateResult = await client.annotate({
        route: location.pathname,
        sid: pending.sid,
        selected_text: pending.text,
        body,
        author: null,
        kind: ui.kind.value,
      });
      switch (res.tag) {
        case "Ok":
          ui.status.textContent = `saved → ${res.source_file}:${res.line}`;
          // Live reload will re-render with the note; close the popup.
          setTimeout(hide, 900);
          break;
        case "NotFound":
          ui.status.textContent = "couldn't map the selection back to source";
          break;
        case "Error":
          ui.status.textContent = `error: ${res.message}`;
          break;
      }
    } catch (err) {
      ui.status.textContent = `failed: ${String(err)}`;
    }
  };

  console.log("[dodeca-annotate] ready");
}

connect()
  .then(main)
  .catch((err) => console.error("[dodeca-annotate] failed to connect:", err));
