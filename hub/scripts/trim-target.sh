#!/usr/bin/env bash
# Keep hub/target from growing without bound.
#
# Cargo never deletes stale builds by itself: each dependency bump, toolchain
# update or new feature set leaves old artifacts behind. This script checks
# the size of hub/target (shared by the core workspace and the Tauri shell)
# and, when it is over the limit, removes the incremental caches first and
# then, if that is not enough, everything.
#
# Usage: scripts/trim-target.sh [limit-in-GB]   (default 15, or $HUB_TARGET_LIMIT_GB)
set -euo pipefail

hub_dir="$(cd "$(dirname "$0")/.." && pwd)"
target="$hub_dir/target"
limit_gb="${1:-${HUB_TARGET_LIMIT_GB:-15}}"

size_gb() { du -sk "$target" 2>/dev/null | awk '{ printf "%d", $1 / 1024 / 1024 }'; }

if [ ! -d "$target" ]; then
  echo "No $target yet; nothing to trim."
  exit 0
fi

size="$(size_gb)"
echo "hub/target is ${size} GB (limit ${limit_gb} GB)."
[ "$size" -le "$limit_gb" ] && exit 0

echo "Removing incremental caches..."
find "$target" -maxdepth 3 -type d -name incremental -prune -exec rm -rf {} +
size="$(size_gb)"
echo "hub/target is now ${size} GB."
[ "$size" -le "$limit_gb" ] && exit 0

echo "Still over the limit; removing hub/target entirely (next build starts from scratch)."
rm -rf "$target"
