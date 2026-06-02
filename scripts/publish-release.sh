#!/bin/bash
# Publish dodeca release artifacts to the Scaleway Object Storage bucket the
# installer reads from (see install.sh / xtask/src/ci.rs RELEASE_BASE_URL).
#
# Usage: scripts/publish-release.sh vX.Y.Z [dist-dir]
# Run after scripts/release.sh has populated dist/ (tarballs + installers +
# SHA256SUMS).
#
# Requires:
#   - aws CLI (Scaleway is S3-compatible; we pass --endpoint-url)
#   - a Scaleway Object Storage key with write access to the `bearcove-dist`
#     bucket (fr-par), exported as AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY.
#
# Layout (must match install.sh's BASE_URL = .../dodeca/releases):
#   s3://bearcove-dist/dodeca/releases/<version>/dodeca-<platform>.tar.xz
#   s3://bearcove-dist/dodeca/releases/<version>/SHA256SUMS
#   s3://bearcove-dist/dodeca/releases/latest      (text file: the version string)
#   s3://bearcove-dist/dodeca/install.sh           (stable installer, curl|sh)
#   s3://bearcove-dist/dodeca/install.ps1          (stable installer, irm|iex)
set -euo pipefail

VERSION="${1:?Usage: $0 vX.Y.Z [dist-dir]}"
DIST="${2:-dist}"

ENDPOINT="https://s3.fr-par.scw.cloud"
BUCKET="bearcove-dist"
REL="dodeca/releases/$VERSION"

LINUX="dodeca-x86_64-unknown-linux-gnu.tar.xz"
MACOS="dodeca-aarch64-apple-darwin.tar.xz"
for f in "$LINUX" "$MACOS"; do
  if [ ! -f "$DIST/$f" ]; then
    echo "ERROR: missing $DIST/$f — run scripts/release.sh first" >&2
    exit 1
  fi
done

s3() { aws s3 --endpoint-url "$ENDPOINT" "$@"; }

# Upload $1 to bucket key $2 as a public object, optionally with content-type $3.
put() {
  local src="$1" key="$2" ctype="${3:-}"
  if [ -n "$ctype" ]; then
    s3 cp "$src" "s3://$BUCKET/$key" --acl public-read --content-type "$ctype"
  else
    s3 cp "$src" "s3://$BUCKET/$key" --acl public-read
  fi
}

echo "Publishing dodeca $VERSION -> s3://$BUCKET/$REL/"
put "$DIST/$LINUX" "$REL/$LINUX"
put "$DIST/$MACOS" "$REL/$MACOS"
[ -f "$DIST/SHA256SUMS" ] && put "$DIST/SHA256SUMS" "$REL/SHA256SUMS" "text/plain"

# Stable, version-independent installer URLs so the curl one-liner never moves.
[ -f "$DIST/dodeca-installer.sh" ]  && put "$DIST/dodeca-installer.sh"  "dodeca/install.sh"  "text/x-shellscript"
[ -f "$DIST/dodeca-installer.ps1" ] && put "$DIST/dodeca-installer.ps1" "dodeca/install.ps1" "text/plain"

# Flip `latest` LAST: an interrupted upload must never advertise a broken release.
printf '%s' "$VERSION" | s3 cp - "s3://$BUCKET/dodeca/releases/latest" \
  --acl public-read --content-type "text/plain"

# Objects are public-read; the same S3 API endpoint serves them anonymously.
WEB="https://$BUCKET.s3.fr-par.scw.cloud"
echo "Done."
echo "  latest  -> $VERSION"
echo "  verify  -> curl -fsSL $WEB/dodeca/releases/latest"
echo "  install -> curl -fsSL $WEB/dodeca/install.sh | sh"
