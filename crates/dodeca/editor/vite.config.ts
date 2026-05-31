import { defineConfig } from "vite";

// The editor is served at /_/edit/* by dodeca's http cell. The server-rendered
// shell (/_dodeca/edit/<page>) loads /_/edit/edit.js + /_/edit/edit.css, so we
// emit those stable names; chunks/workers get hashed and are served from the
// same dir.
export default defineConfig({
  base: "/_/edit/",
  worker: { format: "es" },
  // monaco-languageclient drives Monaco through the VS Code API. We use the
  // codegame editor-api as the `monaco` namespace (no vanilla monaco-editor),
  // and alias any stray `monaco-editor` import onto it so only one copy loads.
  resolve: {
    dedupe: ["@codingame/monaco-vscode-editor-api", "@codingame/monaco-vscode-api", "vscode"],
    alias: { "monaco-editor": "@codingame/monaco-vscode-editor-api" },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    rollupOptions: {
      input: "src/main.ts",
      output: {
        entryFileNames: "edit.js",
        chunkFileNames: "[name]-[hash].js",
        assetFileNames: (info) =>
          info.name && info.name.endsWith(".css")
            ? "edit.css"
            : "[name]-[hash][extname]",
      },
    },
  },
});
