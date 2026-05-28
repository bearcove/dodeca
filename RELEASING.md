# Releasing dodeca

Releases are currently **driven manually** from artifacts built on the
self-hosted Forgejo runners (`code.vixen.rs`). The previous fully-automated
GitHub Actions release job has been retired; its generator code still lives in
`xtask/src/ci.rs` but is no longer invoked by any workflow.

End users install via the `releases/latest` redirect (see `README.md`), so
publishing a GitHub release for a new tag is what makes that version "the latest"
for both the curl installer and the Homebrew tap.

## How versioning works

The `ddc` version is **not** read from `Cargo.toml` (which is intentionally
stale). It comes from `DODECA_RELEASE_VERSION`, which CI sets only on tag builds;
`dodeca_version()` strips a leading `v`. So a `v0.14.3` tag produces
`ddc 0.14.3`. A build from a plain `main` push reports the stale `Cargo.toml`
version — never release those artifacts.

## Prerequisites

- `fj` CLI authenticated against `code.vixen.rs` (or a `FJ_TOKEN` env var with a
  Forgejo token). The token also lives in `~/vixenware/.envrc` as
  `FORGEJO_TOKEN`.
- `gh` authenticated against `github.com` with push access to
  `bearcove/dodeca` and `bearcove/homebrew-tap`.

## Steps

Replace `vX.Y.Z` with the new version throughout.

### 1. Tag and push

The tag must be pushed to **both** remotes: Forgejo builds it; GitHub needs it to
exist for `gh release create`.

```bash
git tag -a vX.Y.Z -m "dodeca vX.Y.Z"
git push forgejo vX.Y.Z
git push origin vX.Y.Z
```

### 2. Wait for the Forgejo tag build

```bash
export FJ_TOKEN=...   # Forgejo token
fj --host code.vixen.rs -R bearcove/dodeca run list      # find the run id for ref vX.Y.Z
fj --host code.vixen.rs -R bearcove/dodeca run watch <run-id>
```

The build asserts `ddc --version == ddc X.Y.Z`, so a green run already confirms
the version stamp.

### 3. Download the archives

Each run uploads two artifacts: `build-linux-x64` and `build-macos-arm64`. Each
is a zip wrapping a single `.tar.xz`. (The API's `size_in_bytes` field is
misleading — trust the actual extracted file.)

```bash
mkdir -p /tmp/ddc-rel && cd /tmp/ddc-rel

# Get the artifact ids for the run:
fj --host code.vixen.rs -R bearcove/dodeca --json \
  api repos/bearcove/dodeca/actions/runs/<run-id>/artifacts

# Download + unzip each (replace <id> with the ids above):
for id in <linux-id> <macos-id>; do
  curl -fsSL -H "Authorization: token $FJ_TOKEN" \
    "https://code.vixen.rs/api/v1/repos/bearcove/dodeca/actions/artifacts/$id/zip" \
    -o "art-$id.zip"
  unzip -o -q "art-$id.zip"
done
```

You should now have:

- `dodeca-x86_64-unknown-linux-gnu.tar.xz`
- `dodeca-aarch64-apple-darwin.tar.xz`

### 4. Create the GitHub release

```bash
cp /path/to/dodeca/install.sh ./dodeca-installer.sh
gh release create vX.Y.Z -R bearcove/dodeca \
  --title "dodeca vX.Y.Z" \
  --generate-notes \
  dodeca-x86_64-unknown-linux-gnu.tar.xz \
  dodeca-aarch64-apple-darwin.tar.xz \
  dodeca-installer.sh
```

The release becomes `latest` automatically, so
`releases/latest/download/dodeca-installer.sh` immediately serves it.

### 5. Update the Homebrew tap

`scripts/update-homebrew.sh` reads the archives from a `dist/` directory and
pushes a regenerated formula to `bearcove/homebrew-tap`.

```bash
mkdir -p dist && cp dodeca-*.tar.xz dist/
export HOMEBREW_TAP_TOKEN="$(gh auth token)"   # needs push access to the tap
bash /path/to/dodeca/scripts/update-homebrew.sh vX.Y.Z
```

## Notes

- `website.yml` (GitHub Pages docs deploy) is unrelated to releases — it just
  installs the published binary via the curl installer and builds the docs site.
- If/when releases should be automated again from Forgejo, the GitHub
  `build_ci_workflow` release job in `xtask/src/ci.rs` is the template to port
  into `build_forgejo_workflow`.
