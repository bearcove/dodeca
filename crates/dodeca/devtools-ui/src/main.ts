import "./devtools.css";

const editorRoot = document.getElementById("vixen-editor");

if (editorRoot) {
  import("./editor")
    .then(({ mountEditor }) => mountEditor(editorRoot))
    .catch((err) => {
      console.error("[dodeca-devtools] editor failed:", err);
      const statusEl = editorRoot.querySelector(".vx-status");
      if (statusEl) statusEl.textContent = `failed: ${String(err)}`;
    });
} else {
  import("./page")
    .then(({ mountPageDevtools }) => mountPageDevtools())
    .catch((err) => console.error("[dodeca-devtools] page shell failed:", err));
  import("./annotate")
    .then(({ mountAnnotate }) => mountAnnotate())
    .catch((err) => console.error("[dodeca-devtools] annotate failed:", err));
}
