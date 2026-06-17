# Monolith binary exploration

This note explores collapsing dodeca from host + dynamically loaded cells into
one statically linked `ddc` binary.

## Current source-of-truth shape

The documentation still describes the older separate-process cell model, but
the source has already moved past that. The current runtime shape is:

- `crates/dodeca/src/host.rs` owns typed cell client caching.
- `crates/dodeca/src/cell_loader.rs` lazily `dlopen`s
  `libddc_cell_<name>.{dylib,so,dll}`.
- `crates/dodeca-cell-runtime/src/lib.rs` exports the per-cell
  `dodeca_cell_vtable_v1` symbol and starts a cell runtime thread after FFI
  attach.
- `crates/dodeca/src/cells.rs` is the main facade used by the build and serve
  code. Most of dodeca calls helpers there rather than opening clients itself.
- `xtask/src/ci.rs` and `xtask/src/main.rs` still know how to discover, build,
  install, verify, and package cell cdylibs.

That means the next collapse is not "processes to threads"; that already
happened. The next collapse is "runtime-loaded service islands to ordinary Rust
modules/libraries linked into `ddc`."

## Target shape

The target should be one `ddc` artifact for the host platform:

- No `libloading`.
- No `dodeca-cell-runtime`.
- No `libddc_cell_*` release artifacts.
- No internal vox-ffi links for build pipeline transformations.
- No proto crates whose only job is host-to-cell serialization.
- Keep vox for external boundaries that are actually RPC boundaries, such as
  browser devtools/websocket paths, if those remain.

The implementation boundary should become ordinary Rust APIs:

- `crates/dodeca/src/cells.rs` becomes a temporary compatibility facade, then
  gets renamed or dissolved into domain modules such as `markdown`, `html`,
  `assets`, `search`, `templates`, and `serve_ui`.
- Former `cell-*-proto` result enums either disappear in favor of domain
  errors, or move next to the code that still benefits from those shaped
  outputs.
- Former host callbacks become local traits or closures. For example:
  - gingembre rendering gets a local `TemplateLoader` and `DataResolver`;
  - HTML inline CSS/JS processing calls the CSS and JS modules directly;
  - TUI commands use the existing `Host` channels directly.

This removes the most confusing part of the current system: everything is
already in one process, but still behaves like a plugin network.

## Module survival pass

Keep the product feature surface. The point of the monolith is to remove the
deployment/runtime cell boundary, not to use the migration as a hidden feature
diet.

Surviving feature modules:

- `gingembre`: template rendering is core. It should be a normal template
  module using the existing `gingembre` crate directly.
- `markdown`: markdown/frontmatter/highlighting/source maps are core authoring
  path.
- `html`: hotmeal-based DOM rewriting, link extraction, code button injection,
  wiki/internal link resolution, image transforms, and minification belong in
  core.
- `css`: URL rewriting and minification are on the dev=prod path.
- `js`: URL rewriting through parsed JS is on the dev=prod path.
- `image`: decode/resize/thumbhash is core to responsive images.
- `webp`: optimized raster output path.
- `jxl`: keep format support; any future format-policy change should be explicit
  and independent from the monolith work.
- `fonts`: fontcull subsetting is explicitly part of the dodeca promise.
- `data`: loading structured data is core.
- `search`: built-in search is a product feature.
- `linkcheck`: internal and external link checking are correctness machinery.
- `sass`: documented asset pipeline feature.
- `svgo`: SVG optimization remains part of asset handling.
- `vite`: the integration may still spawn Vite/node externally, but it does not
  need to be a dynamically loaded internal module.
- `http`: serve/devtools plumbing, folded into server modules.
- `tui`: local task/module wired to existing host channels.
- `dialoguer`: CLI prompting support, called directly where needed.
- `term`: terminal recording support for authoring/rendered code blocks.
- `code-execution`: docs/code-sample execution support; keep its sandboxing
  boundary explicit even after removing the cell boundary.
- `minify`: keep the feature, but merge the implementation into the HTML module
  if the standalone service crate is just deployment residue.
- `html-diff`: keep live-reload diffing if still needed, but place it at the
  hotmeal/live-reload boundary rather than preserving a cell-shaped module.

Only architecture scaffolding is on the cut list:

- `lifecycle` proto/service wiring.
- `host-proto` as an internal cell callback protocol.
- `dodeca-cell-runtime`.
- `libloading` and `vox-ffi` usage for internal build pipeline calls.
- cell cdylib discovery/build/install/release code in `xtask`.

The earlier concern about shedding some diagram rendering belongs in `marq`,
where those handlers live. Dodeca's markdown module should follow marq's public
handler surface rather than deciding diagram-handler policy during this
monolith migration.

`marq` and `tracey` are expected to be absorbed into dodeca later, but not as
part of this pass. During the monolith collapse they should remain external
crates with stable call sites. The collapse should remove the cell/runtime
deployment boundary first; ecosystem absorption is a later source-tree and
ownership move.

## Migration shape

The least risky path is to keep behavior observable and collapse one boundary at
a time:

1. Add direct Rust APIs for callback-free modules first.
   Good pilots: `svgo`, `image`, `webp`, `css`, `js`, `sass`, `data`.
   These mostly expose an impl type already and do not need reverse host calls.
   Started with `svgo` and `data`: both crates now expose `rlib` outputs, keep
   their dynamic-cell exports behind a default feature, and are consumed by
   dodeca with that feature disabled.
   Continued with the main asset processors: `css`, `js`, `sass`, `image`,
   `webp`, `jxl`, and `fonts` now follow the same pattern and are called
   directly from the `cells.rs` facade.

2. Replace `crates/dodeca/src/cells.rs` helpers to call direct APIs for the
   pilot modules while preserving the current helper function names. That keeps
   the production build paths as the oracle.

3. Move callback-heavy modules next.
   - `gingembre`: replace RPC-backed template/data/function callbacks with
     local loader/resolver/function adapters.
   - `html`: replace callback-to-host CSS/JS processing with direct calls into
     the CSS/JS modules.
   - `tui`/`http`: wire directly to host/server channels instead of opening
     internal services.

4. Once no helper goes through `Host::client_async`, delete the dynamic
   infrastructure:
   - `crates/dodeca/src/cell_loader.rs`
   - `crates/dodeca-cell-runtime`
   - internal `CellClient` caching in `Host`
   - `libloading`, `vox-ffi`, and internal `transport-ffi` usage
   - cell cdylib discovery/build/install/release code in `xtask`

5. Rename or remove the old `cells/` tree. The surviving code should become
   ordinary crates/modules named for product domains rather than deployment
   mechanics.

## Verification oracle

Use existing production-like paths instead of unit-test rewrites:

- `cargo check -p ddc -p dodeca`
- focused integration runs through `crates/integration-tests`, especially:
  - `rendered_markdown`
  - `templates`
  - `sass`
  - `static_assets`
  - `cache_busting`
  - `search`
  - `internal_links`
  - `dead_links`
  - `code_execution`, only if that module is still in the survival set
- a real `ddc build --no-tui` on a fixture or known site after the dynamic
  loader is gone.

The key invariant: dev mode must still use the same production transformations.
Collapsing cells is not permission to create a dev shortcut path.
