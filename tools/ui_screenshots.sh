#!/usr/bin/env bash
# Off-screen screenshots of the app; see tools/ui_screenshots.py.
set -euo pipefail
out=${1:?usage: tools/ui_screenshots.sh OUT_PREFIX}
here=$(cd "$(dirname "$0")" && pwd)
scratch=$(mktemp -d)
mkdir -p "$scratch/wallforge"
python3 - "$scratch/wallforge/config.json" <<'PY'
import json, os, sys
src = os.path.join(os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")), "wallforge/config.json")
d = json.load(open(src)) if os.path.exists(src) else {}
d.update(outputs={}, rotate_minutes=0, autostart=False, check_updates=False, auto_update=False)
json.dump(d, open(sys.argv[1], "w"))
PY
if pgrep -f "^python3 -m wallforge" >/dev/null; then echo "close WallForge first (it is single-instance)" >&2; exit 1; fi
gtk4-broadwayd :7 >/dev/null 2>&1 & bpid=$!
trap 'kill $bpid 2>/dev/null; rm -rf "$scratch"' EXIT
sleep 1
env -u WAYLAND_DISPLAY -u DISPLAY GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 XDG_CONFIG_HOME="$scratch" \
    timeout 60 python3 "$here/ui_screenshots.py" "$out"
