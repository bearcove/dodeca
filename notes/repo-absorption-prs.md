# Repository Absorption PR Triage

This tracks open pull requests in first-wave repositories absorbed into Dodeca.
Tracey is intentionally deferred because it has a larger active PR queue.

## Imported Repositories

- `bearcove/marq`
  - Open PRs: `#19` release-plz.
  - Decision: automation release PR, no code carry-over needed before archival.

- `bearcove/hotmeal`
  - Open PRs: `#39` release-plz.
  - Decision: automation release PR, no code carry-over needed before archival.

- `bearcove/pikru`
  - Open PRs: `#12` release-plz.
  - Decision: automation release PR, no code carry-over needed before archival.

- `bearcove/aasvg-rs`
  - Open PRs: none.

- `bearcove/svag`
  - Open PRs: `#5` "Fix compatibility when encoding is enabled in quick-xml via feature unification".
  - Decision: preserved by merging the filtered PR head into this Dodeca branch as `Port svag PR #5 quick-xml compatibility fix`.

- `bearcove/picante`
  - Open PRs: `#45` "Define Picante semantics (spec) + Tracey coverage", plus release-plz `#55`.
  - Decision: release-plz is automation noise. `#45` is stale against the imported release branch; it contains older config/version/runtime churn and should not be blindly merged. The Picante spec material is already present in the imported release subtree, but the PR should be closed or superseded after a focused Dodeca-native review.

- `bearcove/fontcull`
  - Open PRs: `#16` "Migrate from clap to facet-args", plus release-plz `#21`.
  - Decision: release-plz is automation noise. `#16` is stale against current `main` and would remove newer docs/changelog/library work. Carry the intent forward as a fresh Dodeca-local task: remove `clap` from `libs/fontcull/fontcull-cli` when the CLI surface is revisited.

## Deferred

- `bearcove/tracey`
  - Defer absorption until its active PR queue is handled separately.
