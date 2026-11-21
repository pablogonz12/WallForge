#!/bin/bash
# update.sh — update an installed WallForge to the latest main, in one line:
#
#   curl -fsSL https://raw.githubusercontent.com/pgm1207/WallForge/main/update.sh | bash
#
# Downloads the current main branch into a temporary folder, installs it over ~/.local/opt/wallforge (rebuilding the
# native renderer) and restarts a running session. Options pass through to install.sh, e.g.
#   curl -fsSL .../update.sh | bash -s -- --no-build
set -euo pipefail
REPO="${WALLFORGE_REPO:-https://github.com/pgm1207/WallForge}"
REF="${WALLFORGE_REF:-main}"

if ! command -v git >/dev/null; then
  echo "update.sh: git is required (it fetches the source)." >&2; exit 1
fi
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# The installed copy keeps its own .git so updates are small; clone or fetch that, and fall back to a fresh clone.
dest="${WALLFORGE_PREFIX:-$HOME/.local/opt/wallforge}"
if [ -d "$dest/.git" ]; then
  echo "Updating $dest..."
  git -C "$dest" remote set-url origin "$REPO"
  if ! git -C "$dest" fetch -q origin "$REF" || ! git -C "$dest" merge -q --ff-only FETCH_HEAD; then
    echo "Fast-forward failed (local changes?); downloading a fresh copy instead."
    git clone -q --depth 1 --branch "$REF" "$REPO" "$tmp/src"
    rm -rf "$dest"
    [ -d "$tmp/src" ] && mv "$tmp/src" "$dest"
  fi
else
  echo "Downloading $REPO ($REF)..."
  git clone -q --depth 1 --branch "$REF" "$REPO" "$tmp/src"
  dest="$tmp/src"
fi

exec bash "$dest/install.sh" "$@"
