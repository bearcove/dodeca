import { defineConfig } from "vite";

// The unified DevTools UI is served at /_/devtools/* by dodeca's http cell.
// Normal pages load the page shell; /_dodeca/edit/<page> loads the editor mode.
// Chunks/workers are hashed and served from the same namespace.
export default defineConfig({
  base: "/_/devtools/",
  worker: { format: "es" },
  // monaco-vscode-api relies on syntax that the minifier mangles (breaks its
  // module/version identity), and on assets not being inlined.
  esbuild: { minifySyntax: false },
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
    cssCodeSplit: true,
    assetsInlineLimit: 0,
    rollupOptions: {
      input: "src/main.ts",
      output: {
        entryFileNames: "devtools.js",
        chunkFileNames: "chunk/[name]-[hash].js",
        assetFileNames: (info) =>
          info.name === "devtools.css" || info.name === "main.css"
            ? "devtools.css"
            : info.name && info.name.endsWith(".css")
              ? "chunk/[name]-[hash][extname]"
            : "[name]-[hash][extname]",
      },
    },
  },
});
