import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import { session, voxServiceMetadata } from "@bearcove/vox-core";
import { wsConnector } from "@bearcove/vox-ws";
import { DevtoolsServiceClient } from "./devtools.generated";
import "./editor.css";

// Markdown editing only needs the core editor worker.
(self as unknown as { MonacoEnvironment: monaco.Environment }).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};

const root = document.getElementById("vixen-editor");
if (!root) throw new Error("#vixen-editor mount point missing");
const route = root.dataset.route ?? "/";
const token = root.dataset.token ?? "";

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/_/ws`;
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

  const status = (text: string) => {
    statusEl.textContent = text;
  };

  status("connecting…");
  const established = await session.initiator(wsConnector(wsUrl()), {
    metadata: voxServiceMetadata("DevtoolsService"),
  });
  const client = new DevtoolsServiceClient(established.rootConnection().caller());

  const loaded = await client.editLoad(token, route);
  if (loaded.tag !== "Ok") {
    status(loaded.tag === "Denied" ? "not authorized to edit" : "no editable page here");
    return;
  }
  const sourceKey = loaded.source_key;

  const editor = monaco.editor.create(editorEl, {
    value: loaded.content,
    language: "markdown",
    automaticLayout: true,
    wordWrap: "on",
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
  });
  status("loaded");
  saveBtn.disabled = false;

  // Ctrl/Cmd-S saves too.
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => void save());

  saveBtn.addEventListener("click", () => void save());

  async function save(): Promise<void> {
    saveBtn.disabled = true;
    status("saving…");
    try {
      const result = await client.editSave(token, sourceKey, editor.getValue());
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
  }
}

main(root).catch((err) => {
  console.error(err);
  const statusEl = root?.querySelector(".vx-status");
  if (statusEl) statusEl.textContent = `failed: ${String(err)}`;
});
