"""Tell the desktop's palette generator about the wallpaper so colours follow it.

Backends: noctalia, matugen, wallust, pywal (wal), or a custom command ({image} placeholder).
"""
from __future__ import annotations

import shlex
import shutil
import subprocess
from pathlib import Path

from . import library

CACHE = Path.home() / ".cache/wallforge"
BACKENDS = ["auto", "noctalia", "matugen", "wallust", "pywal", "custom", "none"]


def still_for(item: library.Item) -> Path | None:
    """An image palette tools can read: the preview, or a frame of an animated one."""
    prev = item.preview
    if prev and prev.suffix.lower() in (".jpg", ".jpeg", ".png", ".webp"):
        return prev
    if prev:  # gif preview: extract first frame
        out = CACHE / f"{item.id}.png"
        if not out.exists() and shutil.which("ffmpeg"):
            CACHE.mkdir(parents=True, exist_ok=True)
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(prev), "-frames:v", "1", str(out)], timeout=30)
        return out if out.exists() else None
    return None


def detect() -> str:
    for exe, name in (("noctalia", "noctalia"), ("matugen", "matugen"), ("wallust", "wallust"), ("wal", "pywal")):
        if shutil.which(exe):
            return name
    return "none"


def available() -> list[str]:
    return [n for e, n in (("noctalia", "noctalia"), ("matugen", "matugen"), ("wallust", "wallust"), ("wal", "pywal")) if shutil.which(e)]


def _cmd(backend: str, img: Path, output: str | None, custom: str) -> list[str] | None:
    if backend == "noctalia":
        return ["noctalia", "msg", "wallpaper-set"] + ([output] if output else []) + [str(img)]
    if backend == "matugen":
        return ["matugen", "image", str(img)]
    if backend == "wallust":
        return ["wallust", "run", str(img)]
    if backend == "pywal":
        return ["wal", "-n", "-q", "-i", str(img)]
    if backend == "custom" and custom.strip():
        return [part.replace("{image}", str(img)) for part in shlex.split(custom)]
    return None


def sync(cfg, item: library.Item, output: str | None):
    backend = cfg.data.get("palette_backend", "auto")
    if backend == "auto":
        backend = detect()
    if backend in ("none", ""):
        return
    img = still_for(item)
    if not img:
        return
    cmd = _cmd(backend, img, output, cfg.data.get("palette_command", ""))
    if not cmd:
        return
    try:
        subprocess.run(cmd, capture_output=True, timeout=20)
    except Exception:
        pass
