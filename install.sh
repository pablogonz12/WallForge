#!/bin/bash
# Install the menu entry and icon for the current user. Run from the repository folder.
set -e
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps" "$data/icons/hicolor/symbolic/apps"
sed "s|@EXEC@|$here/wallforge.sh|" "$here/data/dev.wallforge.Wallforge.desktop.in" > "$data/applications/dev.wallforge.Wallforge.desktop"
cp "$here/assets/wallforge.svg" "$data/icons/hicolor/scalable/apps/dev.wallforge.Wallforge.svg"
cp "$here/assets/wallforge-symbolic.svg" "$data/icons/hicolor/symbolic/apps/dev.wallforge.Wallforge-symbolic.svg"
gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true
if [ -f "$here/render/Cargo.toml" ] && command -v cargo >/dev/null; then
  echo "Building the native renderer (beta)..."
  (cd "$here/render" && cargo build --release -q) && echo "Native renderer built." || echo "Native renderer build failed (optional)."
fi
echo "Installed. Find WallForge in your app menu."
