+++
title = "JavaScript & Vite"
weight = 40
+++

## Static JavaScript

JavaScript files in `static/` are included as-is (with cache-busted filenames). Reference them in your templates:

```html
<script src="/js/main.js"></script>
```

dodeca rewrites the `src` path to the content-hashed version automatically.

## Vite integration

dodeca has first-class Vite support. If a `vite.config.*` file is detected in your project root, dodeca integrates automatically.

### Dev mode (`ddc serve`)

dodeca starts the Vite dev server and proxies requests to it. You get Vite's HMR alongside dodeca's live reload for content changes.

### Production (`ddc build`)

dodeca runs `pnpm run build`, reads Vite's manifest file, and rewrites all asset paths in your HTML to point to the built files with correct cache-busted URLs.

### Setup

1. Initialize your frontend project:

    ```bash
    pnpm init
    pnpm add -D vite
    ```

2. Create a `vite.config.js`:

    ```js
    export default {
        build: {
            manifest: true,
            rollupOptions: {
                input: "src/main.js"
            }
        }
    };
    ```

3. Reference your entry point in a template:

    ```html
    <script type="module" src="/src/main.js"></script>
    ```

dodeca handles the rest — detecting the config, starting/building Vite, and rewriting paths.
