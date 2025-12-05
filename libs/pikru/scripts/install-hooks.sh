#!/bin/bash
#
# Install git hooks for pikru development
#
# This script sets up pre-commit hooks that automatically update
# the visual comparison HTML file when committing changes.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
HOOKS_DIR="$PROJECT_DIR/.git/hooks"

echo "🔧 Installing pikru git hooks..."

# Check if we're in a git repository
if [ ! -d "$PROJECT_DIR/.git" ]; then
    echo "❌ Error: Not in a git repository"
    exit 1
fi

# Create hooks directory if it doesn't exist
mkdir -p "$HOOKS_DIR"

# Install pre-commit hook
echo "📝 Installing pre-commit hook..."
cp "$SCRIPT_DIR/pre-commit" "$HOOKS_DIR/pre-commit"
chmod +x "$HOOKS_DIR/pre-commit"

echo "✅ Pre-commit hook installed successfully"
echo ""
echo "The pre-commit hook will:"
echo "  - Regenerate comparison.html before each commit"
echo "  - Add the updated comparison.html to your commit"
echo "  - Ensure visual comparisons are always up-to-date"
echo ""
echo "🚀 Git hooks installation complete!"
echo ""
echo "To test the hook, try making a commit:"
echo "  git commit -m 'test commit'"
