# Markdown Extension Providers

## Motivation

Dodeca should let a site register domain-specific documentation transforms
without baking that domain into Dodeca itself.

The immediate example is Vixen's standard-library docs. Vixen already has
declaration-only stdlib headers and resolver/LSP machinery that can resolve
symbols such as `exec`, `Host.which`, and `Tree` to real source spans. Vixen
docs should be able to ask for those declarations by symbol identity and embed
the current source, without copying signatures into markdown by hand.

The general feature is not "Vixen preprocessing". It is Dodeca extensibility:
websites should be able to provide functionality that turns some markdown
surface into other markdown or HTML during the normal Dodeca build.

## Provider Boundary

Providers are third-party programs owned by the site or project being
documented. They are not Dodeca cells by default.

Two transport shapes are enough:

- One-shot CLI: Dodeca invokes a command with CLI arguments and treats the
  command's complete stdout as the transform result. Stderr is diagnostic
  output. A non-zero exit status turns into a page build error.
- Persistent Vox service: Dodeca connects to a typed Vox protocol for providers
  that are expensive to start or need richer request/response structure.

Do not default to JSON-over-stdin. If a provider is persistent, use Vox. If it
is one-shot, keep the contract simple: CLI args in, whole stdout out.

The one-shot path is intentionally less expressive. It can cover "turn this
symbol query into a markdown/code excerpt" without inventing a mini RPC
protocol. Dynamic dependency reporting, structured diagnostics, multiple output
kinds, and caching hints belong naturally in the Vox protocol.

## Markdown Surfaces

Dodeca should expose a few small hook points instead of one generic preprocessor.

Likely first hooks:

- fenced code block language
- link scheme
- possibly inline code tag, if there is a clean authoring syntax later

For Vixen stdlib declaration excerpts, a fenced block is a good fit because the
author wants a code-shaped result:

````markdown
```vxstd
fn:exec
```
````

For links to symbol definitions or source pages, a link scheme is a good fit:

```markdown
[`exec`](vxstd:fn:exec)
[`Host.which`](vxstd:fn:Host.which)
```

Dodeca should not know what `fn:exec` means. It only routes the `vxstd` code
block or link target to the registered provider.

## Configuration Sketch

The exact Styx shape is open, but the configuration needs to distinguish the
markdown surface from the provider transport.

One-shot command sketch:

```styx
markdown_extensions {
  code_block "vxstd" {
    command ("vx" "docs" "stdlib-quote" "{body}")
    output "markdown"
  }

  link_scheme "vxstd" {
    command ("vx" "docs" "stdlib-link" "{target}" "{label}")
    output "html"
  }
}
```

Persistent Vox sketch:

```styx
markdown_extensions {
  code_block "vxstd" {
    vox {
      command ("vx" "docs" "provider")
      service "vixen.docs.MarkdownExtension"
    }
  }
}
```

The command templates above are only illustrative. The important contract is
that one-shot providers receive ordinary CLI arguments and return their entire
replacement text on stdout.

## Vox Protocol Shape

A persistent provider should get typed requests. The request should include the
source page path, the markdown surface being transformed, the body or target,
and enough context for useful diagnostics.

The response should be able to express:

- replacement content as markdown or HTML
- files that should be tracked as dependencies
- diagnostics with source spans when available
- head injections, if the transform needs CSS or JavaScript

This is where dynamic dependency tracking belongs. For example, a Vixen stdlib
symbol provider can report that `fn:exec` depends on
`crates/vx-stdlib/std/build.vx`, so changing that header invalidates the
affected page.

## Vixen Stdlib Provider

The Vixen side should own symbol identity, lookup, ambiguity errors, source
span slicing, and formatting.

Example queries:

```text
fn:exec
fn:Host.which
type:Tree
namespace:Tree
```

Kind prefixes matter because stdlib names can overlap. `Tree` can refer to a
type or a namespace. A kind-less query can be allowed only when it resolves
unambiguously.

The provider should slice from parser/resolver spans, not file line ranges. It
can offer render modes such as:

- bare declaration
- namespace-wrapped member declaration
- markdown code block
- HTML code block with source link

## Figue-Generated Documentation

The same extension point should cover CLI documentation generated from figue.
The point is to stop manually writing down CLI help, arguments, flags,
subcommands, environment variables, and config fields.

There are two plausible ownership models:

- Figue exposes a builtin markdown/documentation renderer for any `Facet`
  CLI/config schema.
- Figue exposes a richer typed documentation model, and project-specific docs
  providers decide how to render it for Dodeca.

The second shape is probably more durable. Plain markdown generation is useful,
but a structured model lets Dodeca or a site theme render command reference
tables, examples, provenance/layer information, and nested subcommands without
committing figue itself to one visual style.

A minimal first cut can still be one-shot:

```styx
markdown_extensions {
  code_block "figue-cli" {
    command ("vx" "docs" "cli-reference" "{body}")
    output "markdown"
  }
}
```

Authored docs:

````markdown
```figue-cli
vx
```
````

The provider returns generated markdown for the requested CLI surface. Later, a
persistent Vox provider can return structured docs, dependencies, and richer
diagnostics.

## Open Design Questions

- What is the smallest stable authoring syntax for embedding generated docs
  without making markdown ugly?
- Should one-shot command providers support only replacement stdout, or also an
  optional depfile path for dynamic dependencies?
- Should Dodeca normalize provider output through markdown rendering, or allow
  providers to explicitly choose markdown vs trusted HTML?
- How should provider failures render in serve mode: full page error, inline
  diagnostic block, or both?
- How much context should Dodeca pass to one-shot command providers without
  sliding back into an ad hoc RPC protocol?
