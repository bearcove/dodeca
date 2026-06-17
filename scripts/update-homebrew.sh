#!/bin/bash
# Update homebrew tap with new release
# Usage: scripts/update-homebrew.sh <version>
# Example: scripts/update-homebrew.sh v0.2.12
# Requires: HOMEBREW_TAP_TOKEN environment variable
set -euo pipefail

VERSION="${1:?Usage: $0 <version>}"
VERSION_NUM="${VERSION#v}"  # Strip 'v' prefix if present

REPO="bearcove/dodeca"
TAP_REPO="bearcove/homebrew-tap"

echo "Updating homebrew tap for version $VERSION_NUM..."

# Calculate SHA256 checksums for each platform
echo "Calculating checksums..."
DARWIN_ARM64_SHA=$(sha256sum "dist/dodeca-aarch64-apple-darwin.tar.xz" 2>/dev/null | cut -d' ' -f1 || echo "")
DARWIN_X86_64_SHA=$(sha256sum "dist/dodeca-x86_64-apple-darwin.tar.xz" 2>/dev/null | cut -d' ' -f1 || echo "")
LINUX_ARM64_SHA=$(sha256sum "dist/dodeca-aarch64-unknown-linux-gnu.tar.xz" 2>/dev/null | cut -d' ' -f1 || echo "")
LINUX_X86_64_SHA=$(sha256sum "dist/dodeca-x86_64-unknown-linux-gnu.tar.xz" 2>/dev/null | cut -d' ' -f1 || echo "")

echo "  darwin-arm64:  ${DARWIN_ARM64_SHA:-not built}"
echo "  darwin-x86_64: ${DARWIN_X86_64_SHA:-not built}"
echo "  linux-arm64:   ${LINUX_ARM64_SHA:-not built}"
echo "  linux-x86_64:  ${LINUX_X86_64_SHA:-not built}"

# Verify we have at least one platform
if [[ -z "$DARWIN_ARM64_SHA" && -z "$DARWIN_X86_64_SHA" && -z "$LINUX_ARM64_SHA" && -z "$LINUX_X86_64_SHA" ]]; then
  echo "ERROR: No platform artifacts found in dist/"
  exit 1
fi

# Generate the formula
cat > dodeca.rb << 'FORMULA_START'
class Dodeca < Formula
  desc "A fully incremental static site generator"
  homepage "https://github.com/bearcove/dodeca"
  version "VERSION_NUM_PLACEHOLDER"
  license any_of: ["MIT", "Apache-2.0"]

FORMULA_START

# Add macOS section if we have macOS builds
if [[ -n "$DARWIN_ARM64_SHA" || -n "$DARWIN_X86_64_SHA" ]]; then
  cat >> dodeca.rb << 'MACOS_START'
  on_macos do
MACOS_START

  if [[ -n "$DARWIN_ARM64_SHA" ]]; then
    cat >> dodeca.rb << DARWIN_ARM
    on_arm do
      url "https://github.com/${REPO}/releases/download/${VERSION}/dodeca-aarch64-apple-darwin.tar.xz"
      sha256 "${DARWIN_ARM64_SHA}"
    end
DARWIN_ARM
  fi

  if [[ -n "$DARWIN_X86_64_SHA" ]]; then
    cat >> dodeca.rb << DARWIN_X86
    on_intel do
      url "https://github.com/${REPO}/releases/download/${VERSION}/dodeca-x86_64-apple-darwin.tar.xz"
      sha256 "${DARWIN_X86_64_SHA}"
    end
DARWIN_X86
  elif [[ -n "$DARWIN_ARM64_SHA" ]]; then
    # Fallback to ARM for Intel Macs (Rosetta)
    cat >> dodeca.rb << DARWIN_FALLBACK
    on_intel do
      url "https://github.com/${REPO}/releases/download/${VERSION}/dodeca-aarch64-apple-darwin.tar.xz"
      sha256 "${DARWIN_ARM64_SHA}"
    end
DARWIN_FALLBACK
  fi

  cat >> dodeca.rb << 'MACOS_END'
  end

MACOS_END
fi

# Add Linux section if we have Linux builds
if [[ -n "$LINUX_ARM64_SHA" || -n "$LINUX_X86_64_SHA" ]]; then
  cat >> dodeca.rb << 'LINUX_START'
  on_linux do
LINUX_START

  if [[ -n "$LINUX_ARM64_SHA" ]]; then
    cat >> dodeca.rb << LINUX_ARM
    on_arm do
      url "https://github.com/${REPO}/releases/download/${VERSION}/dodeca-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "${LINUX_ARM64_SHA}"
    end
LINUX_ARM
  fi

  if [[ -n "$LINUX_X86_64_SHA" ]]; then
    cat >> dodeca.rb << LINUX_X86
    on_intel do
      url "https://github.com/${REPO}/releases/download/${VERSION}/dodeca-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "${LINUX_X86_64_SHA}"
    end
LINUX_X86
  fi

  cat >> dodeca.rb << 'LINUX_END'
  end

LINUX_END
fi

# Add install and test sections
cat >> dodeca.rb << 'FORMULA_END'
  def install
    bin.install "ddc"
  end

  test do
    system "#{bin}/ddc", "--version"
  end
end
FORMULA_END

# Replace placeholder with actual version (perl -i is portable across GNU/BSD)
perl -i -pe "s/VERSION_NUM_PLACEHOLDER/${VERSION_NUM}/g" dodeca.rb

echo "Generated dodeca.rb"

# Clone the tap repo, update, and push
echo "Updating homebrew tap..."
WORK_DIR=$(mktemp -d)
trap "rm -rf '$WORK_DIR'" EXIT

git clone "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/${TAP_REPO}.git" "$WORK_DIR/tap"
cp dodeca.rb "$WORK_DIR/tap/Formula/dodeca.rb"

cd "$WORK_DIR/tap"
git config user.name "github-actions[bot]"
git config user.email "github-actions[bot]@users.noreply.github.com"
git add Formula/dodeca.rb
git commit -m "dodeca ${VERSION_NUM}"
git push

echo "Homebrew tap updated successfully!"
