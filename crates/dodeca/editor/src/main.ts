import * as monaco from "@codingame/monaco-vscode-editor-api";
import { LogLevel } from "@codingame/monaco-vscode-api";
import type { ILogger } from "@codingame/monaco-vscode-log-service-override";
import {
  RegisteredFileSystemProvider,
  RegisteredMemoryFile,
  registerFileSystemOverlay,
} from "@codingame/monaco-vscode-files-service-override";
import { EditorApp, type EditorAppConfig } from "monaco-languageclient/editorApp";
import {
  MonacoVscodeApiWrapper,
  type MonacoVscodeApiConfig,
} from "monaco-languageclient/vscodeApiWrapper";
import {
  defineDefaultWorkerLoaders,
  useWorkerFactory,
} from "monaco-languageclient/workerFactory";
import { MonacoLanguageClient } from "monaco-languageclient";
import { CloseAction, ErrorAction } from "vscode-languageclient/browser.js";
import {
  AbstractMessageReader,
  AbstractMessageWriter,
  type DataCallback,
  type Disposable,
  type Message,
  type MessageReader,
  type MessageWriter,
} from "vscode-jsonrpc";
import { connectLane, voxServiceMetadata, channel } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import initHotmeal, { diff_html, apply_patches_json_on_element } from "hotmeal-wasm";
import { DevtoolsServiceClient, type EditEntry } from "./devtools.generated";
import { ArboriumHighlighter } from "./highlight";
// @ts-expect-error — monaco-vim ships no types; initVimMode(editor, statusNode) → { dispose() }.
import { initVimMode } from "monaco-vim";
import "./editor.css";

const root = document.getElementById("vixen-editor");
if (!root) throw new Error("#vixen-editor mount point missing");
const initialRoute = root.dataset.route ?? "/";
const token = root.dataset.token ?? "";

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
}

/** Canonical route key: strip trailing slashes, keep "/" for the root. */
function normalizeRoute(route: string): string {
  return route.replace(/\/+$/, "") || "/";
}

/** Parse a full HTML document string into { head, body } inner-HTML strings. */
function splitDocument(html: string): { head: string; body: string } {
  const doc = new DOMParser().parseFromString(html, "text/html");
  return { head: doc.head.innerHTML, body: doc.body.innerHTML };
}

function configureWorkerFactory(logger?: ILogger): void {
  const loaders = defineDefaultWorkerLoaders();
  loaders.TextMateWorker = undefined;
  loaders.extensionHostWorkerMain = undefined;
  useWorkerFactory({ workerLoaders: loaders, logger });
}

/** Server → browser: each vox `Rx<string>` chunk is one JSON-RPC message. */
class VoxMessageReader extends AbstractMessageReader implements MessageReader {
  private callback: DataCallback | undefined;
  constructor(private readonly rx: AsyncIterable<string>) {
    super();
    void this.pump();
  }
  listen(callback: DataCallback): Disposable {
    this.callback = callback;
    return { dispose: () => (this.callback = undefined) };
  }
  private async pump(): Promise<void> {
    try {
      for await (const text of this.rx) {
        try {
          this.callback?.(JSON.parse(text) as Message);
        } catch (err) {
          this.fireError(err instanceof Error ? err : new Error(String(err)));
        }
      }
    } catch (err) {
      this.fireError(err instanceof Error ? err : new Error(String(err)));
    }
    this.fireClose();
  }
}

/** Browser → server: serialize each message to one vox `Tx<string>` chunk. */
class VoxMessageWriter extends AbstractMessageWriter implements MessageWriter {
  constructor(private readonly tx: { send(s: string): Promise<void>; close(): void }) {
    super();
  }
  async write(msg: Message): Promise<void> {
    await this.tx.send(JSON.stringify(msg));
  }
  end(): void {
    this.tx.close();
  }
}

async function main(mount: HTMLElement): Promise<void> {
  mount.innerHTML = `
    <div class="vx-toolbar">
      <button class="vx-tree-toggle" title="Toggle file tree">☰</button>
      <span class="vx-route"></span>
      <span class="vx-spacer"></span>
      <button class="vx-vim-toggle vx-iconbtn" title="Toggle Vim mode">vim</button>
      <button class="vx-theme-toggle vx-iconbtn" title="Toggle dark mode">🌙</button>
      <span class="vx-status"></span>
      <input class="vx-commitmsg" type="text" placeholder="commit message (optional)" />
      <button class="vx-save" disabled>Save</button>
    </div>
    <div class="vx-body vx-tree-collapsed">
      <div class="vx-tree"></div>
      <div class="vx-editorcol">
        <div class="vx-tabs"></div>
        <div class="vx-editor"></div>
        <div class="vx-vim-status"></div>
      </div>
      <div class="vx-preview"></div>
    </div>
  `;
  const bodyEl = mount.querySelector(".vx-body") as HTMLElement;
  const routeEl = mount.querySelector(".vx-route") as HTMLElement;
  const statusEl = mount.querySelector(".vx-status") as HTMLElement;
  const saveBtn = mount.querySelector(".vx-save") as HTMLButtonElement;
  const treeToggle = mount.querySelector(".vx-tree-toggle") as HTMLButtonElement;
  const commitMsgEl = mount.querySelector(".vx-commitmsg") as HTMLInputElement;
  const themeToggle = mount.querySelector(".vx-theme-toggle") as HTMLButtonElement;
  const vimToggle = mount.querySelector(".vx-vim-toggle") as HTMLButtonElement;
  const treeEl = mount.querySelector(".vx-tree") as HTMLElement;
  const tabsEl = mount.querySelector(".vx-tabs") as HTMLElement;
  const editorColEl = mount.querySelector(".vx-editorcol") as HTMLElement;
  const editorEl = mount.querySelector(".vx-editor") as HTMLElement;
  const vimStatusEl = mount.querySelector(".vx-vim-status") as HTMLElement;
  const previewEl = mount.querySelector(".vx-preview") as HTMLElement;
  const status = (text: string) => (statusEl.textContent = text);

  // Tree collapses by default; the toolbar button toggles it.
  treeToggle.addEventListener("click", () => bodyEl.classList.toggle("vx-tree-collapsed"));

  // Preview lives in a same-origin <iframe> so it renders the full page exactly
  // as served: the template's <head> (main.css, fonts) plus the injected syntax
  // CSS / copy-button / search assets / build-info, AND the page's own client JS
  // runs (e.g. data-timestamp date formatting, components). No sandbox: this is
  // the user's own trusted, same-origin content and we WANT its scripts to run.
  const previewFrame = document.createElement("iframe");
  previewFrame.className = "vx-preview-frame";
  previewEl.appendChild(previewFrame);
  // Document inside the iframe; only valid after a srcdoc load has settled.
  const previewBody = (): HTMLElement | null =>
    previewFrame.contentDocument?.body ?? null;

  // 1. Connect over the devtools websocket and open the DevtoolsService lane.
  status("connecting…");
  const client = await connectLane(wsConnector(wsUrl()), DevtoolsServiceClient, {
    laneMetadata: voxServiceMetadata("DevtoolsService"),
  });

  // 2. Load the page list + the initial page.
  const list = await client.editList(token);
  const entries: EditEntry[] = list.tag === "Ok" ? list.entries : [];
  const loaded = await client.editLoad(token, initialRoute);
  if (loaded.tag !== "Ok") {
    status(loaded.tag === "Denied" ? "not authorized to edit" : "no editable page here");
    return;
  }
  // 3. Boot the VS Code API (classic mode, no extension host).
  const vscodeApiConfig: MonacoVscodeApiConfig = {
    $type: "classic",
    viewsConfig: { $type: "EditorService", htmlContainer: editorEl },
    logLevel: LogLevel.Warning,
    monacoWorkerFactory: configureWorkerFactory,
    advanced: { loadExtensionServices: false },
  };
  await new MonacoVscodeApiWrapper(vscodeApiConfig).start();

  // 4. File-system provider so go-to-definition / opening any page works. Every
  // page's content is read from the live db via edit_read (pre-fetched).
  const fsProvider = new RegisteredFileSystemProvider(false);
  const contents = await Promise.all(
    entries.map(async (e) => {
      const r = await client.editRead(token, e.uri);
      return {
        uri: e.uri,
        content: r.tag === "Ok" ? r.content : "",
        base: r.tag === "Ok" ? r.base : "",
      };
    }),
  );
  for (const { uri, content } of contents) {
    fsProvider.registerFile(new RegisteredMemoryFile(monaco.Uri.parse(uri), content));
  }
  registerFileSystemOverlay(1, fsProvider);

  // 5. Editor on the initial page.
  const editorAppConfig: EditorAppConfig = {
    codeResources: { modified: { text: loaded.content, uri: loaded.uri } },
    editorOptions: { wordWrap: "on", minimap: { enabled: false }, automaticLayout: true },
    languageDef: {
      languageExtensionConfig: {
        id: "markdown",
        extensions: [".md"],
        aliases: ["Markdown", "markdown"],
        mimetypes: ["text/markdown"],
      },
    },
  };
  const editorApp = new EditorApp(editorAppConfig);
  await editorApp.start(editorEl);
  const editor = editorApp.getEditor();

  // 5b. Light/dark theme: follows the OS by default, with a manual override
  //     persisted in localStorage. The Monaco theme is switched in lockstep with
  //     the shell's `data-theme` (which drives the CSS palette + arborium colors).
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)");
  let manualTheme = localStorage.getItem("vx-theme"); // "light" | "dark" | null
  const themeNow = (): "light" | "dark" =>
    manualTheme === "light" || manualTheme === "dark"
      ? manualTheme
      : prefersDark.matches
        ? "dark"
        : "light";
  const applyTheme = (theme: "light" | "dark") => {
    mount.dataset.theme = theme;
    monaco.editor.setTheme(theme === "dark" ? "vs-dark" : "vs");
    themeToggle.textContent = theme === "dark" ? "☀️" : "🌙";
  };
  applyTheme(themeNow());
  themeToggle.addEventListener("click", () => {
    manualTheme = themeNow() === "dark" ? "light" : "dark";
    localStorage.setItem("vx-theme", manualTheme);
    applyTheme(manualTheme as "light" | "dark");
  });
  prefersDark.addEventListener("change", () => {
    if (!manualTheme) applyTheme(themeNow());
  });

  // 5c. Optional Vim mode (monaco-vim), persisted across reloads.
  let vimMode: { dispose(): void } | null = null;
  const setVim = (on: boolean) => {
    if (on && !vimMode && editor) {
      try {
        vimMode = initVimMode(editor, vimStatusEl) as { dispose(): void };
        editorColEl.classList.add("vx-vim");
        vimToggle.classList.add("vx-on");
      } catch (err) {
        console.error("vim mode failed:", err);
        return;
      }
    } else if (!on && vimMode) {
      vimMode.dispose();
      vimMode = null;
      editorColEl.classList.remove("vx-vim");
      vimToggle.classList.remove("vx-on");
    }
    localStorage.setItem("vx-vim", on ? "1" : "");
  };
  if (localStorage.getItem("vx-vim")) setVim(true);
  vimToggle.addEventListener("click", () => setVim(!vimMode));

  // 6. LSP over a vox channel (in-process Backend on the host).
  const [c2sTx, c2sRx] = channel<string>();
  const [s2cTx, s2cRx] = channel<string>();
  void client.lsp(token, c2sRx, s2cTx);
  const languageClient = new MonacoLanguageClient({
    name: "vixen-authoring",
    clientOptions: {
      documentSelector: ["markdown"],
      errorHandler: {
        error: () => ({ action: ErrorAction.Continue }),
        closed: () => ({ action: CloseAction.DoNotRestart }),
      },
    },
    messageTransports: {
      reader: new VoxMessageReader(s2cRx),
      writer: new VoxMessageWriter(c2sTx),
    },
  });
  await languageClient.start();

  // 7. Tabs — one Monaco model per opened file, each with its own dirty state
  //    and preview baseline. Switching tabs is `editor.setModel`, so unsaved
  //    edits in other tabs are preserved (no destructive code-resource swap).
  const loadedByUri = new Map(contents.map((c) => [c.uri, { content: c.content, base: c.base }]));
  interface Tab {
    entry: EditEntry;
    model: monaco.editor.ITextModel;
    baseline: string; // last-saved content; drives the dirty marker
    base: string; // on-disk blob oid at load/last-save; conflict-detection token
    prevHead?: string; // last preview <head> inner HTML; re-srcdoc when it changes
    prevBody?: string; // last preview <body> inner HTML, for hotmeal diffing
    highlighter: ArboriumHighlighter; // incremental tree-sitter highlighting
  }
  const tabs = new Map<string, Tab>();
  const tabOrder: string[] = [];
  let activeUri = "";

  const modelFor = (uri: string, content: string): monaco.editor.ITextModel => {
    const u = monaco.Uri.parse(uri);
    return monaco.editor.getModel(u) ?? monaco.editor.createModel(content, "markdown", u);
  };
  const activeTab = (): Tab | undefined => tabs.get(activeUri);
  const isDirty = (t: Tab): boolean => t.model.getValue() !== t.baseline;

  const renderTabs = () => {
    tabsEl.innerHTML = "";
    for (const uri of tabOrder) {
      const tab = tabs.get(uri);
      if (!tab) continue;
      const el = document.createElement("div");
      el.className = "vx-tab" + (uri === activeUri ? " vx-active" : "");
      el.title = tab.entry.route;
      const label = document.createElement("span");
      label.className = "vx-tab-label";
      label.textContent = (isDirty(tab) ? "● " : "") + tab.entry.title;
      label.addEventListener("click", () => activate(uri));
      const close = document.createElement("button");
      close.className = "vx-tab-close";
      close.textContent = "×";
      close.title = "Close tab";
      close.addEventListener("click", (e) => {
        e.stopPropagation();
        closeTab(uri);
      });
      el.append(label, close);
      tabsEl.appendChild(el);
    }
  };

  const renderTree = () => {
    treeEl.innerHTML = "";
    for (const e of entries) {
      const item = document.createElement("button");
      const tab = tabs.get(e.uri);
      item.className =
        "vx-tree-item" +
        (e.uri === activeUri ? " vx-active" : "") +
        (tab && isDirty(tab) ? " vx-dirty" : "");
      item.textContent = e.title;
      item.title = e.route;
      item.addEventListener("click", () => void openTab(e));
      treeEl.appendChild(item);
    }
  };

  // Scroll sync: the render returns a data-sid → source-line map, so we can map
  // the editor's top line to a preview element and vice-versa. `lineIndex` is
  // rebuilt (in source-line order) after each preview render.
  let lineIndex: Array<{ el: HTMLElement; line: number }> = [];
  const rebuildLineIndex = (map: Array<{ sid: string; line: number }>) => {
    const sidToLine = new Map(map.map((s) => [s.sid, s.line]));
    const doc = previewFrame.contentDocument;
    if (!doc) {
      lineIndex = [];
      return;
    }
    lineIndex = [...doc.querySelectorAll<HTMLElement>("[data-sid]")]
      .map((el) => ({ el, line: sidToLine.get(el.dataset.sid ?? "") ?? -1 }))
      .filter((x) => x.line > 0)
      .sort((a, b) => a.line - b.line);
  };

  // One side drives at a time; a short cooldown stops the reciprocal scroll
  // event from echoing back.
  let scrollSource: "editor" | "preview" | null = null;
  let scrollReset: number | undefined;
  const releaseScrollSoon = () => {
    if (scrollReset) clearTimeout(scrollReset);
    scrollReset = setTimeout(() => (scrollSource = null), 120) as unknown as number;
  };
  // The preview scrolls inside its own iframe document, so the geometry is taken
  // from the iframe's window/document rather than the wrapper element.
  const previewWin = (): Window | null => previewFrame.contentWindow;
  const syncPreviewToEditor = () => {
    const win = previewWin();
    if (!editor || lineIndex.length === 0 || !win) return;
    const ranges = editor.getVisibleRanges();
    if (ranges.length === 0) return;
    const topLine = ranges[0].startLineNumber;
    let i = 0;
    while (i + 1 < lineIndex.length && lineIndex[i + 1].line <= topLine) i++;
    const cur = lineIndex[i];
    const next = lineIndex[i + 1];
    // Element rects are relative to the iframe's own viewport (top = 0).
    let delta = cur.el.getBoundingClientRect().top;
    if (next && next.line > cur.line) {
      const frac = Math.min(1, Math.max(0, (topLine - cur.line) / (next.line - cur.line)));
      delta += frac * (next.el.getBoundingClientRect().top - cur.el.getBoundingClientRect().top);
    }
    win.scrollBy(0, delta);
  };
  const syncEditorToPreview = () => {
    if (!editor || lineIndex.length === 0) return;
    let target = lineIndex[0];
    for (const item of lineIndex) {
      if (item.el.getBoundingClientRect().top <= 0) target = item;
      else break;
    }
    editor.setScrollTop(editor.getTopForLineNumber(target.line));
  };
  editor?.onDidScrollChange(() => {
    if (scrollSource === "preview") return;
    scrollSource = "editor";
    syncPreviewToEditor();
    releaseScrollSoon();
  });
  // Scroll + click both fire inside the iframe document; (re)attach them
  // whenever a fresh document is loaded via srcdoc.
  const attachPreviewHandlers = () => {
    const win = previewFrame.contentWindow;
    const doc = previewFrame.contentDocument;
    win?.addEventListener("scroll", () => {
      if (scrollSource === "editor") return;
      scrollSource = "preview";
      syncEditorToPreview();
      releaseScrollSoon();
    });
    doc?.addEventListener("click", handlePreviewClick);
  };

  // 8. Split preview — dodeca's real full-page overlay render, shown in the
  //    same-origin iframe. The first render (and any rare head change) loads the
  //    whole document via `srcdoc`, so the page's CSS/fonts/JS load exactly as
  //    served. Per-keystroke updates hotmeal-patch only the iframe's <body> (the
  //    same diff/patch engine as the served-page HMR), so CSS/JS don't reload
  //    and there's no flicker.
  await initHotmeal();
  let previewTimer: number | undefined;
  // Load a full document into the iframe and resolve once it has settled, so the
  // body / scroll listener are ready to use.
  const srcdocOnce = (html: string) =>
    new Promise<void>((resolve) => {
      previewFrame.addEventListener("load", () => resolve(), { once: true });
      previewFrame.srcdoc = html;
    });
  const refreshPreview = async () => {
    const tab = activeTab();
    if (!tab) return;
    const result = await client.editPreview(token, tab.entry.source_key, tab.model.getValue());
    if (result.tag !== "Ok" || tab !== activeTab()) return; // tab switched mid-flight
    const { head, body } = splitDocument(result.html);
    // First render for this tab, or the <head> changed → reload the whole
    // document so styles/scripts re-apply. Otherwise patch the body in place.
    if (tab.prevHead !== head || !previewFrame.contentDocument) {
      await srcdocOnce(result.html);
      if (tab !== activeTab()) return; // tab switched while loading
      attachPreviewHandlers();
    } else {
      const root = previewBody();
      if (root) {
        try {
          apply_patches_json_on_element(diff_html(tab.prevBody ?? "", body), root);
        } catch {
          root.innerHTML = body; // fall back to a full body replace
        }
      }
    }
    tab.prevHead = head;
    tab.prevBody = body;
    rebuildLineIndex(result.source_map);
  };
  const schedulePreview = () => {
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => void refreshPreview(), 250) as unknown as number;
  };

  const activate = (uri: string) => {
    const tab = tabs.get(uri);
    if (!tab) return;
    activeUri = uri;
    if (editor && editor.getModel() !== tab.model) editor.setModel(tab.model);
    routeEl.textContent = tab.entry.route;
    // Switching files → fresh full render (re-srcdoc head + body), not a diff.
    tab.prevHead = undefined;
    tab.prevBody = undefined;
    renderTabs();
    renderTree();
    editor?.focus();
    void refreshPreview();
  };

  const openTab = async (entry: EditEntry) => {
    if (!tabs.has(entry.uri)) {
      let loadedFile = loadedByUri.get(entry.uri);
      if (!loadedFile) {
        const r = await client.editRead(token, entry.uri);
        loadedFile = {
          content: r.tag === "Ok" ? r.content : "",
          base: r.tag === "Ok" ? r.base : "",
        };
      }
      const model = modelFor(entry.uri, loadedFile.content);
      const highlighter = new ArboriumHighlighter(model, monaco);
      void highlighter.start();
      tabs.set(entry.uri, {
        entry,
        model,
        baseline: loadedFile.content,
        base: loadedFile.base,
        highlighter,
      });
      tabOrder.push(entry.uri);
    }
    activate(entry.uri);
  };

  const closeTab = (uri: string) => {
    if (tabOrder.length <= 1) return; // keep at least one tab open
    const idx = tabOrder.indexOf(uri);
    if (idx < 0) return;
    tabOrder.splice(idx, 1);
    tabs.get(uri)?.highlighter.dispose();
    tabs.delete(uri);
    if (activeUri === uri) {
      activate(tabOrder[Math.min(idx, tabOrder.length - 1)]);
    } else {
      renderTabs();
      renderTree();
    }
  };

  // Internal links in the preview open an editor tab rather than navigating the
  // whole editor away. cell-html already resolves wiki / @/ / relative links to
  // canonical site routes during rendering, so we just map the rendered route
  // back to a page and openTab — no href rewriting, no guessing. The listener is
  // attached to the iframe document (re)attached on each srcdoc load.
  const routeToEntry = new Map(entries.map((e) => [normalizeRoute(e.route), e]));
  function handlePreviewClick(ev: Event) {
    const a = (ev.target as HTMLElement | null)?.closest?.("a");
    if (!a) return;
    const href = a.getAttribute("href");
    if (!href) return;
    if (href.startsWith("#")) {
      ev.preventDefault();
      previewFrame.contentDocument?.querySelector(href)?.scrollIntoView({ behavior: "smooth" });
      return;
    }
    let url: URL;
    try {
      url = new URL(href, location.origin);
    } catch {
      ev.preventDefault();
      return;
    }
    if (url.origin === location.origin) {
      const entry = routeToEntry.get(normalizeRoute(url.pathname));
      if (entry) {
        ev.preventDefault();
        void openTab(entry);
        return;
      }
    }
    // External, or an internal route with no editable page (assets, section
    // indexes): open in a new tab so the editor itself stays put.
    ev.preventDefault();
    if (url.protocol === "http:" || url.protocol === "https:") {
      window.open(url.href, "_blank", "noopener");
    }
  }

  (globalThis as unknown as { __vixen: unknown }).__vixen = {
    editor,
    languageClient,
    monaco,
    client,
    tabs,
  };

  // Open the initial page as the first tab (reusing EditorApp's model + the
  // base oid from edit_load, so we don't re-fetch it).
  loadedByUri.set(loaded.uri, { content: loaded.content, base: loaded.base });
  await openTab({
    source_key: loaded.source_key,
    route: loaded.route,
    uri: loaded.uri,
    title: entries.find((e) => e.uri === loaded.uri)?.title ?? loaded.route,
  });

  status("ready");
  saveBtn.disabled = false;
  editor?.onDidChangeModelContent(() => {
    renderTabs(); // refresh dirty markers
    renderTree();
    schedulePreview();
  });

  const save = async (): Promise<void> => {
    const tab = activeTab();
    if (!tab) return;
    saveBtn.disabled = true;
    status("saving…");
    try {
      const value = tab.model.getValue();
      const result = await client.editSave(token, {
        source_key: tab.entry.source_key,
        buffer: value,
        base: tab.base,
        message: commitMsgEl.value.trim(),
      });
      switch (result.tag) {
        case "Ok":
          tab.baseline = value;
          tab.base = result.base; // adopt the new oid for the next save
          commitMsgEl.value = "";
          status(`saved ${result.commit.slice(0, 8)}`);
          renderTabs();
          renderTree();
          break;
        case "Conflict":
          status("conflict: file changed on disk since you opened it — reload to merge");
          break;
        case "Error":
          status(`error: ${result.message}`);
          break;
        default:
          status(result.tag.toLowerCase());
      }
    } catch (err) {
      status(`save failed: ${String(err)}`);
    } finally {
      saveBtn.disabled = false;
    }
  };
  saveBtn.addEventListener("click", () => void save());
  editor?.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => void save());

  // 11. Image upload — paste or drop an image onto the editor; it's stored next
  //     to the page (committed as you) and a markdown ref is inserted at the
  //     cursor. The on-disk file then flows through dodeca's image pipeline.
  const firstImage = (
    items: DataTransferItemList | undefined,
    files: FileList | undefined,
  ): File | null => {
    if (items) {
      for (let i = 0; i < items.length; i++) {
        const it = items[i];
        if (it.kind === "file" && it.type.startsWith("image/")) {
          const f = it.getAsFile();
          if (f) return f;
        }
      }
    }
    if (files) {
      for (let i = 0; i < files.length; i++) {
        if (files[i].type.startsWith("image/")) return files[i];
      }
    }
    return null;
  };

  const uploadImage = async (file: File): Promise<void> => {
    const tab = activeTab();
    if (!tab || !editor) return;
    status(`uploading ${file.name || "image"}…`);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const r = await client.editUpload(token, {
        source_key: tab.entry.source_key,
        filename: file.name || "image.png",
        bytes,
      });
      if (r.tag !== "Ok") {
        status(r.tag === "Error" ? `upload failed: ${r.message}` : r.tag.toLowerCase());
        return;
      }
      const sel = editor.getSelection();
      if (sel) {
        editor.executeEdits("upload", [{ range: sel, text: r.markdown, forceMoveMarkers: true }]);
      }
      editor.focus();
      status(`inserted ${r.path}`);
    } catch (err) {
      status(`upload failed: ${String(err)}`);
    }
  };

  editorEl.addEventListener(
    "paste",
    (e) => {
      const img = firstImage(e.clipboardData?.items, e.clipboardData?.files);
      if (img) {
        e.preventDefault();
        e.stopPropagation();
        void uploadImage(img);
      }
    },
    true,
  );
  editorEl.addEventListener(
    "dragover",
    (e) => {
      if (e.dataTransfer?.types?.includes("Files")) e.preventDefault();
    },
    true,
  );
  editorEl.addEventListener(
    "drop",
    (e) => {
      const img = firstImage(e.dataTransfer?.items, e.dataTransfer?.files);
      if (img) {
        e.preventDefault();
        e.stopPropagation();
        void uploadImage(img);
      }
    },
    true,
  );
}

main(root).catch((err) => {
  console.error(err);
  const statusEl = root?.querySelector(".vx-status");
  if (statusEl) statusEl.textContent = `failed: ${String(err)}`;
});
