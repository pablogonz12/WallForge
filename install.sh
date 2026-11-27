#!/bin/bash
# install.sh — install or update WallForge for the current user.
#
#   ./install.sh              install from this checkout into ~/.local/opt/wallforge
#   ./install.sh --from-git   update a checkout: fetch origin/main there, then install from it
#   ./install.sh --no-build   skip the native renderer build (keeps the installed binary)
#
# The installed copy is the one that runs: the menu entry, the `wallforge` command and the login service all point
# at ~/.local/opt/wallforge. The repository checkout stays the source of truth, so updates only need to copy over
# what changed. Re-running this is always safe; a running session (wallforge.service) is restarted at the end.
set -euo pipefail

PREFIX="${WALLFORGE_PREFIX:-$HOME/.local/opt/wallforge}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
build=1
update=0

for arg in "$@"; do
  case "$arg" in
    --from-git) update=1 ;;
    --no-build) build=0 ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install.sh: unknown option $arg (try --help)" >&2; exit 2 ;;
  esac
done

copy=0
if [ "$here" != "$(readlink -m -- "$PREFIX")" ]; then
  copy=1
  if ! command -v rsync >/dev/null; then
    echo "install.sh: install rsync with your package manager, then rerun this installer." >&2
    exit 1
  fi
fi
if [ "$update" = 1 ] && [ -d "$here/.git" ]; then
  echo "Updating the checkout in $here..."
  git -C "$here" pull --ff-only
fi

echo "Installing into $PREFIX"
mkdir -p "$PREFIX"
# Copy the sources over an existing install, keeping its .git (updates), the built renderer (target/) and caches.
if [ "$copy" = 1 ]; then
  for d in wallforge render data assets gnome-extension docs packaging tests tools; do
    [ -e "$here/$d" ] || continue
    mkdir -p "$PREFIX/$d"
    rsync -a --delete \
      --exclude '.git' --exclude 'target' --exclude '__pycache__' --exclude '.ruff_cache' \
      "$here/$d/" "$PREFIX/$d/"
  done
  for f in pyproject.toml README.md CHANGELOG.md LICENSE SECURITY.md CODE_OF_CONDUCT.md CONTRIBUTING.md install.sh wallforge.sh; do
    [ -e "$here/$f" ] && cp -a "$here/$f" "$PREFIX/$f"
  done
fi
chmod +x "$PREFIX/wallforge.sh"
# Keep a git history in the installed copy so update.sh can fast-forward instead of downloading everything again.
if [ "$(readlink -f "$here")" = "$(readlink -f "$PREFIX")" ] && command -v git >/dev/null && [ ! -d "$PREFIX/.git" ]; then
  origin="$(git -C "$here" config --get remote.origin.url 2>/dev/null || echo https://github.com/pgm1207/WallForge)"
  git -C "$PREFIX" init -q && git -C "$PREFIX" remote add origin "$origin" 2>/dev/null || true
fi

# ── menu entry, icons and the `wallforge` command ──
mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps" "$data/icons/hicolor/symbolic/apps" "$bin"
sed "s|@EXEC@|$PREFIX/wallforge.sh|" "$PREFIX/data/dev.wallforge.Wallforge.desktop.in" > "$data/applications/dev.wallforge.Wallforge.desktop"
cp "$PREFIX/assets/wallforge.svg" "$data/icons/hicolor/scalable/apps/dev.wallforge.Wallforge.svg"
cp "$PREFIX/assets/wallforge-symbolic.svg" "$data/icons/hicolor/symbolic/apps/dev.wallforge.Wallforge-symbolic.svg"
gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true
ln -sf "$PREFIX/wallforge.sh" "$bin/wallforge"

if [ -d "$PREFIX/gnome-extension" ]; then
  mkdir -p "$data/gnome-shell/extensions"
  rm -rf "$data/gnome-shell/extensions/wallforge@pgm1207.github.io"
  cp -r "$PREFIX/gnome-extension/wallforge@pgm1207.github.io" "$data/gnome-shell/extensions/"
  case "${XDG_CURRENT_DESKTOP:-}" in *GNOME*)
    gnome-extensions enable wallforge@pgm1207.github.io 2>/dev/null || true
    echo "GNOME extension installed. On Wayland, log out and back in once so GNOME finds it.";;
  esac
fi

# ── native renderer (video, scene; web wallpapers also work without it) ──
if [ "$build" = 1 ] && [ -f "$PREFIX/render/Cargo.toml" ]; then
  if command -v cargo >/dev/null; then
    echo "Building the native renderer (about a minute the first time)..."
    (cd "$PREFIX/render" && cargo build --release -q) && echo "Native renderer built." \
      || echo "Native renderer build failed; keeping any existing binary. Run 'wallforge --doctor' for details."
  else
    echo "cargo not found: skipping the native renderer build (install rust to build it)."
  fi
fi

# ── keep a running session on the new code (only for the real install, never for a test prefix) ──
[ "$(readlink -f "$PREFIX")" = "$(readlink -f "$HOME/.local/opt/wallforge")" ] || { echo "Installed. Run '$PREFIX/wallforge.sh' or open WallForge in your app menu."; exit 0; }
restarted=""
if command -v systemctl >/dev/null && systemctl --user is-active --quiet wallforge.service 2>/dev/null; then
  systemctl --user restart wallforge.service && restarted="wallforge.service"
elif pgrep -f "wallforge.*--daemon" >/dev/null 2>&1; then
  pkill -f "wallforge.*--daemon" || true
  ("$PREFIX/wallforge.sh" --daemon >/dev/null 2>&1 &)
  restarted="the WallForge daemon"
fi
# The daemon keeps running renderers alive across a restart, so they still run the old binary. --reload replaces
# them one by one with the new one (each new renderer is up before the old one retires, so no black frame).
if [ -n "$restarted" ]; then
  "$PREFIX/wallforge.sh" --reload >/dev/null 2>&1 && echo "Restarted $restarted and reloaded the wallpapers."
fi

echo "Installed. Run 'wallforge' or open WallForge in your app menu."
