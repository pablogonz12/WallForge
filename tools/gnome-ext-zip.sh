#!/usr/bin/env bash
# Build the extensions.gnome.org upload zip for the bundled GNOME Shell extension.
# Usage: tools/gnome-ext-zip.sh [output-dir]   (default: dist/)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
src="$root/gnome-extension/wallforge@pgm1207.github.io"
out=${1:-$root/dist}
mkdir -p "$out"
gnome-extensions pack --force --out-dir "$out" "$src"
echo "$out/wallforge@pgm1207.github.io.shell-extension.zip"
unzip -l "$out/wallforge@pgm1207.github.io.shell-extension.zip"
