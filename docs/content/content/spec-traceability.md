+++
title = "Spec Traceability"
weight = 60
+++

dodeca supports requirement markers in markdown and can check them against
configured implementation files.

## Syntax

Define a requirement with an `r[...]` marker:

```markdown
r[protocol.handshake]

The client MUST send a handshake message within 5 seconds of connecting.
```

That renders as a requirement block with a stable anchor. Code references use
the same marker shape with a verb:

```rust
// r[impl protocol.handshake]
// r[verify protocol.handshake]
```

Supported reference verbs are:

- `impl` — source code implements the rule.
- `verify` — tests or verification code exercise the rule.
- `depends` — code depends on the rule.
- `related` — code is related to the rule without implementing or verifying it.

Versioned rule IDs such as `protocol.handshake+2` make older references show up
as stale instead of invalid.

## Configuration

Coverage scanning is driven by the source's `impls` configuration:

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

`include` files may use any reference verb. `test_include` files may verify
rules, but `impl` references in test files fail validation so implementation
coverage stays tied to production code.

Mounted sources keep their own `impls` configuration. Coverage queries can be
filtered with `source` and `impl` selectors.

## Live API

During authoring, prefer the dev-server endpoints. They use the running
server's hot Dodeca database, so repeated queries do not need a separate Tracey
server or a cold one-shot scan. Use the URL printed by `ddc serve`; the examples
below assume the default `127.0.0.1:4000` binding.

```sh
# Browser view: http://127.0.0.1:4000/_dodeca/coverage/
curl http://127.0.0.1:4000/_dodeca/coverage/nav.md
curl http://127.0.0.1:4000/_dodeca/coverage/status.md
curl 'http://127.0.0.1:4000/_dodeca/coverage/status.md?source=api&impl=rust'
curl 'http://127.0.0.1:4000/_dodeca/coverage/validate.md?threshold=80'
curl http://127.0.0.1:4000/_dodeca/coverage/rule/protocol.handshake.md
```

Every endpoint supports `.md` for model- and human-facing output, `.json` for
typed tooling output, and `.html` for browser-facing output. The root
`/_dodeca/coverage/` URL serves `nav.html`:

- `nav` — Tracey-style navigation map with spec, coverage, and sources views.
- `status` — summary counts and links to the other queries.
- `config` — configured source/impl globs.
- `uncovered` — rules without implementation references.
- `untested` — rules without verification references.
- `unmapped` — code units without requirement references.
- `stale` — references to older versions of current rules.
- `invalid` — references that do not resolve to a known rule.
- `rule/<id>` — one rule with definitions and references.
- `validate` — pass/fail summary, with optional `threshold`.

Percent-encode rule IDs in URLs when needed, for example `+` as `%2B`.

## CLI

Use `ddc coverage` when no dev server is running, especially in scripts and CI:

```sh
ddc agent
ddc agent install
ddc agent install --agent claude-code
ddc agent install --skills-cli --agent '*'
ddc coverage nav .
ddc coverage status .
ddc coverage config . --source api --impl rust
ddc coverage rule protocol.handshake . --source api --impl rust
ddc coverage validate . --threshold 80
```

`ddc agent` prints a bundled Markdown guide for agents and humans who need the
Dodeca mental model before making changes. `ddc agent install` writes a thin
open Agent Skills-compatible `dodeca/SKILL.md` that delegates back to the
current CLI, so installed skills do not need to duplicate the full workflow.
The default install is project-local `.agents/skills/dodeca`; use
`--agent claude-code` for Claude Code's `.claude/skills/dodeca`, `--global` for
user-level installs, and `--skills-cli` to delegate placement to the
`pnpx`/`npx skills` ecosystem CLI.

The CLI defaults to Markdown and accepts `--format json` for the same typed
responses exposed by the live API. `validate` exits non-zero when coverage is
below the threshold, a reference is invalid or stale, or a test file contains an
`impl` reference.
