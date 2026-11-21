"""Which outputs are fully covered by a window (so the daemon can pause that renderer).

Pure decision logic plus a thin niri IPC wrapper. The renderer pauses on SIGUSR2 and
resumes on SIGUSR1 (render/src/main.rs, PAUSED); the daemon calls
``covered_outputs()`` periodically (or on niri event-stream changes) and signals
renderers whose output newly entered / left the set.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess


def _niri_json(what: str):
    if not shutil.which("niri") or not os.environ.get("NIRI_SOCKET"):
        return None
    try:
        r = subprocess.run(["niri", "msg", "--json", what], capture_output=True, text=True, timeout=2)
        return json.loads(r.stdout) if r.returncode == 0 else None
    except (OSError, subprocess.SubprocessError, ValueError):
        return None


def covered_from_niri(outputs, workspaces, windows) -> set[str]:
    """Output names whose active workspace shows a tiled window spanning the whole output.

    Gapped tiles (smaller than the output) do not count: the wallpaper is visible
    around them. A fullscreen window is exactly output-sized. Floating windows and
    windows scrolled out of view never count.
    """
    size = {n: (o["logical"]["width"], o["logical"]["height"]) for n, o in (outputs or {}).items() if o.get("logical")}
    active = {w["id"]: w["output"] for w in workspaces or [] if w.get("is_active") and w.get("output")}
    out: set[str] = set()
    for w in windows or []:
        name = active.get(w.get("workspace_id"))
        if name not in size or w.get("is_floating"):
            continue
        lay = w.get("layout") or {}
        pos, ws = lay.get("tile_pos_in_workspace_view"), lay.get("window_size")
        if not pos or not ws:
            continue
        if abs(pos[0]) < 1 and abs(pos[1]) < 1 and ws[0] >= size[name][0] and ws[1] >= size[name][1]:
            out.add(name)
    return out


def covered_outputs() -> set[str]:
    """Covered output names on the running niri; empty set if niri is unavailable."""
    o, w, win = _niri_json("outputs"), _niri_json("workspaces"), _niri_json("windows")
    if o is None or w is None or win is None:
        return set()
    return covered_from_niri(o, w, win)
