#!/usr/bin/env bash
# Syncs local wiki/ folder to GitHub Wiki remote repository (https://github.com/halpworld/halptask.wiki.git)

set -e

SOURCE_DIR="$(pwd)"
WIKI_DIR="${SOURCE_DIR}/wiki"
TEMP_DIR=$(mktemp -d)
REMOTE_REPO="https://github.com/halpworld/halptask.wiki.git"

echo "🚀 Syncing HalpTask documentation in '${WIKI_DIR}' to GitHub Wiki (${REMOTE_REPO})..."

if [ ! -d "$WIKI_DIR" ]; then
    echo "❌ Error: '${WIKI_DIR}' directory not found!"
    exit 1
fi

# Cleanup temp dir on exit
trap 'rm -rf "$TEMP_DIR"' EXIT

# Clone or init temp wiki repo
if ! git clone "$REMOTE_REPO" "$TEMP_DIR"; then
    echo "ℹ️  Wiki remote repository clone failed (likely not initialized on GitHub web UI yet)."
    cd "$TEMP_DIR"
    git init -b master
    git remote add origin "$REMOTE_REPO"
else
    cd "$TEMP_DIR"
fi

# Copy wiki markdown and images
cp -r "${WIKI_DIR}"/* .

git add .
if git status --porcelain | grep -q .; then
    git commit -m "docs(wiki): sync wiki documentation from main repository"
fi

CURRENT_BRANCH=$(git branch --show-current || echo "master")
echo "📤 Pushing wiki update to remote branch '${CURRENT_BRANCH}'..."

# Push current branch to remote
if git push origin "${CURRENT_BRANCH}"; then
    echo "✅ GitHub Wiki sync completed successfully!"
else
    echo ""
    echo "⚠️  Failed to push to GitHub Wiki remote."
    echo "--------------------------------------------------------------------------------"
    echo "If the wiki repository is not initialized yet on GitHub:"
    echo "  1. Open https://github.com/halpworld/halptask/wiki in your browser"
    echo "  2. Click 'Create the first page' and save any page (e.g. Home)"
    echo "  3. Re-run './scripts/sync_wiki.sh'"
    echo "--------------------------------------------------------------------------------"
    exit 1
fi
