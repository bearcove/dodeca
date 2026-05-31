import * as monaco from "@codingame/monaco-vscode-editor-api";
import { LogLevel } from "@codingame/monaco-vscode-api";
import type { ILogger } from "@codingame/monaco-vscode-log-service-override";
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
import { DevtoolsServiceClient } from "./devtools.generated";
import "./editor.css";

const root = document.getElementById("vixen-editor");
if (!root) throw new Error("#vixen-editor mount point missing");
const route = root.dataset.route ?? "/";
const token = root.dataset.token ?? "";

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
}

/** Monaco workers for "classic" mode (no textmate/extension-host workers). */
function configureWorkerFactory(logger?: ILogger): void {
  const loaders = defineDefaultWorkerLoaders();
  loaders.TextMateWorker = undefined;
  loaders.extensionHostWorkerMain = undefined;
  useWorkerFactory({ workerLoaders: loaders, logger });
}

/**
 * Bridges the LSP JSON-RPC stream onto a vox `Rx<string>` (server → browser):
 * each channel chunk is one JSON-RPC message.
 */
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
    <div class="vx-editor"></div>
  `;
  const routeEl = mount.querySelector(".vx-route") as HTMLElement;
  const statusEl = mount.querySelector(".vx-status") as HTMLElement;
  const saveBtn = mount.querySelector(".vx-save") as HTMLButtonElement;
  const editorEl = mount.querySelector(".vx-editor") as HTMLElement;
  routeEl.textContent = route;
  const status = (text: string) => (statusEl.textContent = text);

  // 1. Connect to the host over the existing devtools websocket. The root
  // connection is the Noop service (handled by the cell); DevtoolsService runs
  // on a secondary connection that the cell proxies to the host.
  status("connecting…");
  const established = await session.initiator(wsConnector(wsUrl()), {
    metadata: voxServiceMetadata("Noop"),
  });
  const devtools = await established
    .handle()
    .openConnection(undefined, voxServiceMetadata("DevtoolsService"));
  const client = new DevtoolsServiceClient(devtools.caller());

  // 2. Load the page's raw markdown + the file URI the LSP keys it by.
  const loaded = await client.editLoad(token, route);
  if (loaded.tag !== "Ok") {
    status(loaded.tag === "Denied" ? "not authorized to edit" : "no editable page here");
    return;
  }
  const { source_key: sourceKey, uri, content } = loaded;

  // 3. Boot the VS Code API + Monaco editor (classic mode).
  const vscodeApiConfig: MonacoVscodeApiConfig = {
    $type: "classic",
    viewsConfig: { $type: "EditorService", htmlContainer: editorEl },
    logLevel: LogLevel.Warning,
    monacoWorkerFactory: configureWorkerFactory,
  };
  await new MonacoVscodeApiWrapper(vscodeApiConfig).start();

  const editorAppConfig: EditorAppConfig = {
    codeResources: { modified: { text: content, uri } },
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

  // 4. Tunnel a Language Server session over vox and hand it to the client.
  const [c2sTx, c2sRx] = channel<string>();
  const [s2cTx, s2cRx] = channel<string>();
  void client.lsp(token, c2sRx, s2cTx); // runs for the session's lifetime
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
  status("ready");
  saveBtn.disabled = false;

  // 5. Save: commit the current buffer as the user.
  const save = async (): Promise<void> => {
    saveBtn.disabled = true;
    status("saving…");
    try {
      const result = await client.editSave(token, sourceKey, editor?.getValue() ?? content);
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
