+++
title = "Vite Integration"
description = "Using Vite for frontend development with dodeca"
weight = 55
+++

Dodeca integrates with [Vite](https://vitejs.dev/) for seamless frontend development. When a Vite configuration is detected, dodeca automatically starts the Vite dev server and proxies requests to it.

## How it works

When you run `ddc serve`, dodeca:

1. Checks for a Vite configuration file at your project root
2. Runs `pnpm install` if needed
3. Starts `pnpm run dev` to launch the Vite dev server
4. Proxies matching requests from the dodeca server to Vite
5. Proxies WebSocket connections for Hot Module Replacement (HMR)

This means you get a single `ddc serve` command that handles both your static site and your frontend assets with full HMR support.

## Setup

### 1. Create Vite configuration

Add a `vite.config.ts` (or `.js`, `.mts`, `.mjs`) at your project root:

```typescript
// vite.config.ts
import { defineConfig } from 'vite'

export default defineConfig({
  // Your Vite configuration
})
```

### 2. Add package.json with dev script

Create a `package.json` with the standard Vite dev script:

```json
{
  "name": "my-site",
  "scripts": {
    "dev": "vite",
    "build": "vite build"
  },
  "devDependencies": {
    "vite": "^5.0.0"
  }
}
```

### 3. Install dependencies

```bash
pnpm install
```

### 4. Run dodeca

```bash
ddc serve
```

You should see output indicating the Vite server is starting:

```
   Starting Vite dev server in /path/to/project
   [vite] VITE v5.x.x ready
   OK Vite dev server running on port 5173
```

## Directory structure

A typical project with Vite might look like:

```
my-site/
├── .config/
│   └── dodeca.yaml
├── content/
│   └── index.md
├── templates/
│   └── base.html.jinja
├── static/                 # Hand-crafted static assets
│   └── favicon.ico
├── dist/                   # Generated assets (Vite build output)
│   └── assets/
│       ├── main-abc123.js
│       └── style-def456.css
├── src/                    # Frontend source (Vite)
│   ├── main.ts
│   └── style.css
├── package.json
├── pnpm-lock.yaml
└── vite.config.ts
```

Note: The `dist/` directory is for generated/build output and should be gitignored. Files in `dist/` take priority over files in `static/` when paths conflict.

## Proxied paths

Dodeca proxies the following requests to Vite:

- `/@vite/*` - Vite client scripts
- `/@id/*` - Module IDs
- `/@fs/*` - File system access
- `/@react-refresh` - React Fast Refresh runtime
- `/node_modules/.vite/*` - Pre-bundled dependencies
- `/node_modules/*` - Node modules
- `*.ts`, `*.tsx`, `*.jsx`, `*.vue`, `*.svelte` - Source files
- `*.hot-update.json`, `*.hot-update.js` - HMR updates

All other requests go through dodeca's normal content handling.

## HMR (Hot Module Replacement)

WebSocket connections for HMR are automatically proxied. This means:

- CSS changes apply instantly without page reload
- JavaScript/TypeScript changes hot-reload (with framework support)
- You get Vite's fast feedback loop while keeping dodeca's live reload for content

## Using in templates

Reference your Vite-built assets in templates. In development, Vite serves them directly:

```html
<!-- In development, Vite serves this with HMR -->
<script type="module" src="/src/main.ts"></script>
```

For production builds, you'll want to use Vite's build output. See the Production section below.

## Production builds

When you run `ddc build`, dodeca automatically:

1. Detects `vite.config.ts` (or variant) at project root
2. Runs `pnpm install` if needed
3. Runs `pnpm run build` to build frontend assets
4. Then proceeds with the normal dodeca build

Configure Vite to output to the `dist/` directory:

```typescript
// vite.config.ts
import { defineConfig } from 'vite'

export default defineConfig({
  build: {
    outDir: 'dist',         // Output to dodeca's dist directory
    manifest: true,         // Generate manifest for asset references
  }
})
```

The built assets in `dist/` are automatically included in dodeca's output. Remember to add `dist/` to your `.gitignore` since these are generated files.

### Dev vs Production assets

You can reference source files directly - dodeca automatically rewrites them to the built output in production:

```html
<!-- Works in both dev and production -->
<script type="module" src="/src/main.ts"></script>
```

In development, Vite serves `/src/main.ts` directly with HMR. In production (`ddc build`), dodeca reads the Vite manifest at `dist/.vite/manifest.json` and automatically rewrites `/src/main.ts` to `/assets/main-abc123.js`.

This means you don't need conditional logic in your templates - the same markup works everywhere.

## Framework examples

### React

```typescript
// vite.config.ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()]
})
```

### Vue

```typescript
// vite.config.ts
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()]
})
```

### Svelte

```typescript
// vite.config.ts
import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()]
})
```

## Troubleshooting

### Vite not starting

- Ensure `vite.config.ts` (or variant) exists at project root
- Check that `pnpm` is installed and in your PATH
- Check that `package.json` has a `dev` script that runs Vite
- Look for error messages in the terminal output

### HMR not working

- Check browser console for WebSocket connection errors
- Ensure the Vite WebSocket path isn't being blocked
- Try refreshing the page to re-establish the connection

### Assets not loading

- Check that the path matches one of the proxied patterns
- Look at the browser Network tab to see where requests are going
- Verify Vite is running by accessing `http://localhost:5173` directly

### Timeout starting Vite

Dodeca waits 30 seconds for Vite to report its port. If your `pnpm install` is slow:

- Pre-run `pnpm install` before starting `ddc serve`
- Consider using `pnpm install --frozen-lockfile` for faster installs
