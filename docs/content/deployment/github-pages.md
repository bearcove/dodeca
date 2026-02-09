+++
title = "GitHub Pages"
weight = 10
+++

dodeca sites can be deployed to GitHub Pages with a simple workflow. The runner only needs the `ddc` binary — no Rust toolchain required.

## GitHub Actions workflow

Create `.github/workflows/deploy.yml`:

```yaml
name: Deploy to GitHub Pages

on:
  push:
    branches: [main]
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install dodeca
        run: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bearcove/dodeca/releases/latest/download/dodeca-installer.sh | sh

      - name: Build site
        run: ddc build

      - name: Upload artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: public/

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

Adjust the `path:` in the upload step to match your `output` setting in `dodeca.styx`.

## Custom domain

For a custom domain, add a `CNAME` file to your `static/` directory and list it in `stable_assets`:

```styx
stable_assets (
    CNAME
)
```

## Path filtering

If your repository contains more than just the site, add path filters to avoid unnecessary builds:

```yaml
on:
  push:
    branches: [main]
    paths:
      - "content/**"
      - "templates/**"
      - "static/**"
      - "sass/**"
      - ".config/**"
```
