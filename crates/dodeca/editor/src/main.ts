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
import { session, voxServiceMetadata, channel } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import { DevtoolsServiceClient, type EditEntry } from "./devtools.generated";
import "./editor.css";

const root = document.getElementById("vixen-editor");
if (!root) throw new Error("#vixen-editor mount point missing");
const initialRoute = root.dataset.route ?? "/";
const token = root.dataset.token ?? "";

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
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
      <span class="vx-route"></span>
      <span class="vx-spacer"></span>
      <span class="vx-status"></span>
      <button class="vx-save" disabled>Save</button>
    </div>
    <div class="vx-body">
      <div class="vx-tree"></div>
      <div class="vx-editor"></div>
      <iframe class="vx-preview" sandbox="allow-same-origin"></iframe>
    </div>
  `;
  const routeEl = mount.querySelector(".vx-route") as HTMLElement;
  const statusEl = mount.querySelector(".vx-status") as HTMLElement;
  const saveBtn = mount.querySelector(".vx-save") as HTMLButtonElement;
  const treeEl = mount.querySelector(".vx-tree") as HTMLElement;
  const editorEl = mount.querySelector(".vx-editor") as HTMLElement;
  const previewEl = mount.querySelector(".vx-preview") as HTMLIFrameElement;
  const status = (text: string) => (statusEl.textContent = text);

  // 1. Connect over the devtools websocket (Noop root + DevtoolsService sub-connection).
  status("connecting…");
  const established = await session.initiator(wsConnector(wsUrl()), {
    metadata: voxServiceMetadata("Noop"),
  });
  const devtools = await established
    .handle()
    .openConnection(undefined, voxServiceMetadata("DevtoolsService"));
  const client = new DevtoolsServiceClient(devtools.caller());

  // 2. Load the page list + the initial page.
  const list = await client.editList(token);
  const entries: EditEntry[] = list.tag === "Ok" ? list.entries : [];
  const loaded = await client.editLoad(token, initialRoute);
  if (loaded.tag !== "Ok") {
    status(loaded.tag === "Denied" ? "not authorized to edit" : "no editable page here");
    return;
  }
  const current = { sourceKey: loaded.source_key, uri: loaded.uri, route: loaded.route };

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
      return { uri: e.uri, content: r.tag === "Ok" ? r.content : "" };
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

  (globalThis as unknown as { __vixen: unknown }).__vixen = { editor, languageClient, monaco, client };

  // 7. File tree.
  const renderTree = () => {
    treeEl.innerHTML = "";
    for (const e of entries) {
      const item = document.createElement("button");
      item.className = "vx-tree-item" + (e.route === current.route ? " vx-active" : "");
      item.textContent = e.title;
      item.title = e.route;
      item.addEventListener("click", () => void open(e));
      treeEl.appendChild(item);
    }
  };

  // 8. Split preview — dodeca's real overlay render of the current buffer.
  let previewTimer: number | undefined;
  const refreshPreview = async () => {
    const result = await client.editPreview(token, current.sourceKey, editor?.getValue() ?? "");
    if (result.tag === "Ok") previewEl.srcdoc = result.html;
  };
  const schedulePreview = () => {
    if (previewTimer) clearTimeout(previewTimer);
    previewTimer = setTimeout(() => void refreshPreview(), 250) as unknown as number;
  };

  // Switch the editor to another page.
  const open = async (entry: EditEntry) => {
    if (entry.route === current.route) return;
    const l = await client.editLoad(token, entry.route);
    if (l.tag !== "Ok") return;
    current.sourceKey = l.source_key;
    current.uri = l.uri;
    current.route = l.route;
    routeEl.textContent = l.route;
    await editorApp.updateCodeResources({ modified: { text: l.content, uri: l.uri } });
    renderTree();
    void refreshPreview();
  };

  routeEl.textContent = current.route;
  renderTree();
  status("ready");
  saveBtn.disabled = false;
  editor?.onDidChangeModelContent(() => schedulePreview());
  void refreshPreview();

  const save = async (): Promise<void> => {
    saveBtn.disabled = true;
    status("saving…");
    try {
      const result = await client.editSave(token, current.sourceKey, editor?.getValue() ?? "");
      switch (result.tag) {
        case "Ok":
          status(`saved ${result.commit.slice(0, 8)}`);
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
}

main(root).catch((err) => {
  console.error(err);
  const statusEl = root?.querySelector(".vx-status");
  if (statusEl) statusEl.textContent = `failed: ${String(err)}`;
});
