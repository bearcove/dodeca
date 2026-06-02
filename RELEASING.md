# Releasing dodeca

Releases are built **and published automatically** by the Forgejo tag build on
`code.vixen.rs/bearcove/dodeca`. Pushing a `vX.Y.Z` tag builds both platforms,
then the `publish` job uploads the archives + installers to the Scaleway
`vixen-misc` bucket the installer reads from. There is no manual download /
`gh release` step anymore.

End users install via the bucket's website endpoint (see `README.md`):

```bash
curl -fsSL https://vixen-misc.s3-website.fr-par.scw.cloud/dodeca/install.sh | sh
```

## How versioning works

The `ddc` version is **not** read from `Cargo.toml` (which is intentionally
stale). It comes from `DODECA_RELEASE_VERSION`, which CI sets only on tag builds;
`dodeca_version()` strips a leading `v`. So a `v0.14.3` tag produces
`ddc 0.14.3`. A build from a plain `main` push reports the stale `Cargo.toml`
version — never release those artifacts. The build job asserts
`ddc --version == ddc X.Y.Z`, so a green run already confirms the version stamp.

## Cutting a release

```bash
git tag -a vX.Y.Z -m "dodeca vX.Y.Z"
git push forgejo vX.Y.Z    # triggers the build + publish
git push origin  vX.Y.Z    # keep the GitHub mirror's tags in sync (optional)
```

Then watch the run:

```bash
export FJ_TOKEN=...        # Forgejo token
fj --host code.vixen.rs -R bearcove/dodeca run list      # find the run for ref vX.Y.Z
fj --host code.vixen.rs -R bearcove/dodeca run watch <run-id>
```

On success the `publish` job has uploaded, under `s3://vixen-misc/`:

- `dodeca/releases/<tag>/dodeca-x86_64-unknown-linux-gnu.tar.xz`
- `dodeca/releases/<tag>/dodeca-aarch64-apple-darwin.tar.xz`
- `dodeca/releases/<tag>/SHA256SUMS`
- `dodeca/install.sh`, `dodeca/install.ps1` — stable, version-independent URLs
- `dodeca/releases/latest` — flipped **last**, so an interrupted upload never
  advertises a broken release

Verify:

```bash
curl -fsSL https://vixen-misc.s3-website.fr-par.scw.cloud/dodeca/releases/latest
DODECA_VERSION=vX.Y.Z curl -fsSL \
  https://vixen-misc.s3-website.fr-par.scw.cloud/dodeca/install.sh | sh
```

## One-time prerequisites

- The Forgejo `bearcove` org (or the `bearcove/dodeca` repo) must have Actions
  secrets `ACCESS_KEY_ID` / `ACCESS_SECRET_KEY` — a Scaleway Object Storage key
  with write access to the `vixen-misc` bucket (the same key `vixen-ci`
  publishes with). The `publish` job maps them to `AWS_*` and `aws s3 cp`s with
  `--acl public-read` through the S3 API endpoint.

## Manual publish (fallback)

If CI is unavailable, download the two `build-*` artifacts from the Forgejo run
into `dist/`, then publish from a checkout:

```bash
bash scripts/release.sh                  # preps dist/ (installers + SHA256SUMS)
AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_DEFAULT_REGION=fr-par \
  bash scripts/publish-release.sh vX.Y.Z
```

`scripts/publish-release.sh` is exactly what the `publish` CI job runs.

## Homebrew (separate channel, still GitHub-based)

`scripts/update-homebrew.sh` still generates a formula whose `url`s point at
`github.com/bearcove/dodeca/releases`. Until that's repointed at the bucket the
`brew` tap is **not** updated by this flow — repoint the formula at the
website endpoint or retire the tap.

## Notes

- `website.yml` (GitHub Pages docs deploy) is unrelated to releases — it just
  installs the published binary via the curl installer and builds the docs site.
- The installer download source lives in `xtask/src/ci.rs`
  (`RELEASE_BASE_URL`); `install.sh` / `install.ps1` are generated from it
  (`cargo xtask ci-github`, `cargo xtask generate-ps1-installer install.ps1`).
