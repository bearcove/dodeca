---
name: dodeca
description: Use when working on a Dodeca site or the Dodeca CLI. This skill delegates to the installed ddc binary so guidance stays current.
compatibility: Compatible with the open Agent Skills standard. Requires ddc on PATH.
metadata:
  source: dodeca-ddc
---

# Dodeca

This installed skill is intentionally small. Dodeca's agent guidance is bundled
inside the `ddc` binary so the CLI can be updated without relying on a stale
copied skill.

Before making claims or edits in a Dodeca project, run:

```sh
ddc agent
```

Then use the current CLI and live server endpoints it points you to. If the site
requires a newer CLI with `site.minimum_ddc_version`, update `ddc` before
changing project code. The most common follow-up commands are:

```sh
ddc diagnostics .
ddc coverage nav .
ddc coverage config .
```

If `ddc serve` is already running, prefer the live Markdown endpoints from the
server URL printed by `ddc serve`:

```sh
curl "$DODECA_URL/_dodeca/coverage/nav.md"
curl "$DODECA_URL/_dodeca/coverage/config.md"
```

Humans can open `$DODECA_URL/_dodeca/coverage/` for the browser navigation
view.

Use `.json` or `--format json` only when a tool needs typed data. Dodeca code
uses Facet and `facet_json`; do not hand-write JSON.

To refresh this installed skill from the current binary, run:

```sh
ddc agent install
ddc agent install --agent claude-code
ddc agent install --skills-cli --agent '*'
```
