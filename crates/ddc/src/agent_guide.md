# Dodeca Agent Guide

If you are an agent working on a Dodeca project, start from the local source and
the running `ddc` process. Do not assume an external Tracey server, Zola dev
semantics, or serde-based JSON contracts.

## Anchor: Zola-Shaped, Different Runtime

Dodeca is a static site generator in the same broad category as Zola: it reads
content, templates, assets, and configuration, then produces a static site.

The important differences are:

- `ddc serve` uses the production pipeline. Dev mode is not a shortcut mode.
- The server keeps a hot Picante database. Prefer live `/_dodeca/...`
  endpoints over repeated cold CLI scans while the server is running.
- Assets are production-shaped during development: cache-busted paths,
  responsive images, font subsetting, search artifacts, and rendered HTML come
  from the same pipeline as `ddc build`.
- Configuration is Styx in `.config/dodeca.styx`, not TOML/YAML.
- Sites can require a minimum CLI with `site.minimum_ddc_version`; if the
  installed `ddc` is too old, config loading fails before the build or serve
  pipeline runs.
- Multi-source sites compose `source {}` blocks through `mounts (...)`;
  source-scoped settings travel with the mounted source.
- Typed output is serialized through Facet and `facet_json`; do not hand-write
  JSON in Dodeca code.

## Anchor: First Commands

Use these before making claims about the project:

```sh
ddc agent
ddc diagnostics .
ddc coverage nav .
ddc coverage config .
```

To install or refresh the thin agent skill from the current binary:

```sh
ddc agent install
ddc agent install --agent claude-code
ddc agent install --global --agent codex
```

The installed skill intentionally delegates back to `ddc agent` so copied skill
files can stay small and stale copies still point to the current CLI.

`ddc agent install` writes an open Agent Skills-compatible `dodeca/SKILL.md`.
The default target is project-local `.agents/skills/dodeca`, which is the common
directory used by the open skills ecosystem and Codex project skills. Use
`--agent claude-code` for `.claude/skills/dodeca`, repeat `--agent` to install
for multiple clients, and add `--global` for the user-level directories. For
the long tail of clients supported by the ecosystem CLI, delegate placement:

```sh
ddc agent install --skills-cli --agent claude-code
ddc agent install --skills-cli --agent '*'
```

That runs `pnpx skills add` when `pnpx` is available, falling back to `npx`.

If a dev server is already running, prefer the live Markdown endpoints:

```sh
curl "$DODECA_URL/_dodeca/coverage/nav.md"
curl "$DODECA_URL/_dodeca/coverage/config.md"
```

`DODECA_URL` is the URL printed by `ddc serve`, commonly
`http://127.0.0.1:4000`. Humans can open
`$DODECA_URL/_dodeca/coverage/` for the browser navigation view.

Use JSON only when a tool needs typed data:

```sh
curl "$DODECA_URL/_dodeca/coverage/nav.json"
ddc coverage nav . --format json
```

## Anchor: Coverage Workflow

Coverage is configured in each source's `impls` block:

```styx
source {
    content content
    impls (
        {
            name rust
            include ("crates/**/*.rs")
            exclude ("target/**")
            test_include ("crates/**/tests/**/*.rs")
        }
    )
}
```

Requirement definitions live in Markdown:

```markdown
r[protocol.handshake]

The client MUST send a handshake first.
```

Code references use verbs:

```rust
// r[impl protocol.handshake]
// r[verify protocol.handshake]
```

Use `source` and `impl` selectors for mounted or multi-implementation sites:

```sh
ddc coverage status . --source api --impl rust
ddc coverage nav . --source api --impl rust
ddc coverage rule protocol.handshake . --source api --impl rust
curl "$DODECA_URL/_dodeca/coverage/rule/protocol.handshake.md?source=api&impl=rust"
```

Important coverage rules:

- `include` files may implement and verify rules.
- `test_include` files may verify rules.
- `r[impl ...]` in `test_include` files fails validation.
- `rule+2` supersedes `rule`; references to older versions are stale, not
  invalid.
- `ddc coverage validate . --threshold N` is the CI/serverless gate.

## Anchor: Live Endpoints

Coverage endpoints support both `.md` and `.json`:

- `/_dodeca/coverage/`
- `/_dodeca/coverage/nav.md`
- `/_dodeca/coverage/status.md`
- `/_dodeca/coverage/config.md`
- `/_dodeca/coverage/uncovered.md`
- `/_dodeca/coverage/untested.md`
- `/_dodeca/coverage/unmapped.md`
- `/_dodeca/coverage/stale.md`
- `/_dodeca/coverage/invalid.md`
- `/_dodeca/coverage/rule/<id>.md`
- `/_dodeca/coverage/validate.md?threshold=N`

For agent reasoning, Markdown is usually the better first view because it
includes navigation and context. Use JSON for scripts, assertions, and typed
comparisons.

## Anchor: Edit Discipline

- Read `.config/dodeca.styx` before changing coverage, mounts, templates, or
  source-scoped behavior.
- Prefer `ddc diagnostics .` for authoring failures.
- Prefer `ddc coverage config .` before deciding which files count for coverage.
- When editing Dodeca itself, start with
  `cargo check -p dodeca -p ddc --all-targets --message-format=short`.
- For server behavior, rebuild `ddc` and exercise the production path through
  focused integration tests with `DODECA_BIN=target/debug/ddc`.
