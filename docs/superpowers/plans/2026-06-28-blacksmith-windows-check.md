# Blacksmith Windows Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a generated Blacksmith Windows compile-check job to dodeca CI without adding Windows packaging or release artifacts.

**Architecture:** `xtask/src/ci.rs` remains the source of truth for `.github/workflows/ci.yml`. The change adds a Windows runner constant and a standalone GitHub-only `windows-check` job that runs before platform artifact jobs and is not included in release dependencies.

**Tech Stack:** Rust xtask generator, GitHub Actions YAML generated through existing workflow structs, Blacksmith GitHub Actions runner label `blacksmith-2vcpu-windows-2025`.

---

## File Structure

- Modify: `xtask/src/ci.rs`
  - Add `GITHUB_WINDOWS_CHECK_RUNNER` constant near the existing runner constants.
  - Add a GitHub-only `windows-check` job in `build_ci_workflow` after `clippy` and before target artifact fan-out.
- Regenerate: `.github/workflows/ci.yml`
  - Generated output only; do not hand-edit.
- Already written spec: `docs/superpowers/specs/2026-06-28-blacksmith-windows-check-design.md`

## Task 1: Add generated Windows check job

**Files:**
- Modify: `xtask/src/ci.rs:127-131`
- Modify: `xtask/src/ci.rs:1150-1183`

- [ ] **Step 1: Add Windows runner constant**

Add this constant below `GITHUB_MACOS_RUNNER`:

```rust
/// Blacksmith Windows runner for cheap compile checks.
const GITHUB_WINDOWS_CHECK_RUNNER: &str = "blacksmith-2vcpu-windows-2025";
```

- [ ] **Step 2: Add generated job after Clippy**

Insert this block immediately after the existing `clippy` job insertion and before `for target in &targets`:

```rust
    if platform == CiPlatform::GitHub {
        jobs.insert(
            "windows-check".to_string(),
            Job::with_runner(RunnerSpec::single(GITHUB_WINDOWS_CHECK_RUNNER).to_runs_on())
                .name("Check Windows")
                .timeout(30)
                .steps([
                    checkout(platform),
                    install_rust(platform),
                    rust_cache(platform, false),
                    Step::run("Check Windows", "cargo check --workspace --all-targets"),
                ]),
        );
    }
```

If `rust_cache(platform, false)` does not exist, use the existing simplest non-target cache helper available in `common` for GitHub jobs. Keep the job standalone: no `needs`, no release dependency push.

- [ ] **Step 3: Run local Rust check for generator compile**

Run:

```bash
cargo check -p xtask
```

Expected: xtask compiles. If helper names are wrong, fix `xtask/src/ci.rs` using existing helper names rather than adding a new abstraction.

## Task 2: Regenerate and inspect GitHub workflow

**Files:**
- Regenerate: `.github/workflows/ci.yml`

- [ ] **Step 1: Regenerate GitHub CI**

Run:

```bash
cargo xtask ci-github
```

Expected: `.github/workflows/ci.yml` is regenerated.

- [ ] **Step 2: Verify generator check**

Run:

```bash
cargo xtask ci-github --check
```

Expected: `ci.yml is up to date.` and `install.sh is up to date.`

- [ ] **Step 3: Inspect generated Windows job**

Read `.github/workflows/ci.yml` and confirm it contains:

```yaml
  windows-check:
    name: Check Windows
    runs-on:
      - blacksmith-2vcpu-windows-2025
```

Also confirm `release.needs` does not include `windows-check`.

## Task 3: Final verification

**Files:**
- Inspect: `.github/workflows/ci.yml`
- Inspect: `xtask/src/ci.rs`

- [ ] **Step 1: Run final generator compile/check**

Run:

```bash
cargo check -p xtask
cargo xtask ci-github --check
```

Expected: both pass.

- [ ] **Step 2: Summarize changed behavior**

Report:

- Windows now gets a Blacksmith compile-check job.
- Windows is not part of release packaging.
- Linux and macOS target artifact jobs are unchanged except for generated YAML ordering if any.
