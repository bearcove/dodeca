# livereload-client

[![MIT + Apache 2.0](https://img.shields.io/badge/license-MIT%20%2B%20Apache%202.0-blue)](./LICENSE-MIT)
[![salsa | yes please](https://img.shields.io/badge/salsa-yes%20please-green)](https://crates.io/crates/salsa)

# livereload-client

WASM client for dodeca live reload. It connects to the devtools server, receives DOM patch messages, and applies them in the browser to update pages without a full reload.

## What it does

- Opens a websocket connection to dodeca devtools
- Receives patch messages encoded with `dodeca-protocol`
- Applies DOM patches using `hotmeal-wasm`

## Usage

This crate is built and bundled by the workspace tooling; you typically don’t depend on it directly.



## Sponsors

Thanks to all individual sponsors:

<p> <a href="https://github.com/sponsors/fasterthanlime">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/github-dark.svg">
<img src="./static/sponsors-v3/github-light.svg" height="40" alt="GitHub Sponsors">
</picture>
</a> <a href="https://patreon.com/fasterthanlime">
    <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/patreon-dark.svg">
    <img src="./static/sponsors-v3/patreon-light.svg" height="40" alt="Patreon">
    </picture>
</a> </p>

...along with corporate sponsors:

<p> <a href="https://aws.amazon.com">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/aws-dark.svg">
<img src="./static/sponsors-v3/aws-light.svg" height="40" alt="AWS">
</picture>
</a> <a href="https://zed.dev">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/zed-dark.svg">
<img src="./static/sponsors-v3/zed-light.svg" height="40" alt="Zed">
</picture>
</a> <a href="https://depot.dev?utm_source=facet">
<picture>
<source media="(prefers-color-scheme: dark)" srcset="./static/sponsors-v3/depot-dark.svg">
<img src="./static/sponsors-v3/depot-light.svg" height="40" alt="Depot">
</picture>
</a> </p>

...without whom this work could not exist.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
