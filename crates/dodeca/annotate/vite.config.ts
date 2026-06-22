import { defineConfig } from "vite";

// The annotation overlay is served at /_/annotate/* by dodeca's http cell and
// injected into dev pages via a <script type="module">. We emit stable
// annotate.js + annotate.css names; any chunks are hashed in the same dir.
export default defineConfig({
  base: "/_/annotate/",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    assetsInlineLimit: 0,
    rollupOptions: {
      input: "src/main.ts",
      output: {
        entryFileNames: "annotate.js",
        chunkFileNames: "chunk/[name]-[hash].js",
        assetFileNames: (info) =>
          info.name && info.name.endsWith(".css") ? "annotate.css" : "[name]-[hash][extname]",
      },
    },
  },
});
