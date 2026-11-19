"""Tell the desktop's palette generator about the wallpaper so colours follow it.

Backends: noctalia, matugen, wallust, pywal (wal), or a custom command ({image} placeholder).
"""
from __future__ import annotations

import shlex
import shutil
import subprocess
from pathlib import Path

from . import engine, library, monitors

CACHE = Path.home() / ".cache/wallforge"
BACKENDS = ["auto", "noctalia", "matugen", "wallust", "pywal", "custom", "none"]


def _screen_size(output: str | None) -> tuple[int, int]:
    try:
        outs = monitors.outputs()
        m = next((o for o in outs if o["name"] == output), outs[0])
        return int(m["width"]), int(m["height"])
    except Exception:
        return 2560, 1440


def rendered_still(cfg, item: library.Item, output: str | None) -> Path | None:
    """A sharp, screen-sized still of the wallpaper itself (the Workshop preview is small and looks blurry when a
    desktop shows it big, e.g. in the niri overview). Cached; None if it cannot be made."""
    w, h = _screen_size(output)
    out = CACHE / f"{item.id}-{w}x{h}.png"
    if out.exists():
        return out
    CACHE.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(".tmp.png")
    try:
        if item.kind == "video":
            vid = item.run_dir / (item.base_project().get("file") or "") if item.base_project().get("file") else None
            if not (vid and vid.is_file()) or not shutil.which("ffmpeg"):
                return None
            for ss in ("5", "0"):
                subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", ss, "-i", str(vid), "-frames:v", "1", "-vf",
                                f"scale={w}:{h}:force_original_aspect_ratio=increase:flags=lanczos,crop={w}:{h}", str(tmp)],
                               timeout=30, capture_output=True)
                if tmp.exists():
                    break
        elif item.kind == "scene" and engine.use_native(cfg, item):
            args = engine.build_native_args(cfg, item, output or "")
            subprocess.run(args + ["--screenshot", str(tmp), "--screenshot-size", f"{w}x{h}", "--screenshot-time", "3",
                                   "--silent"], timeout=40, capture_output=True)
        if tmp.exists() and tmp.stat().st_size > 1000:
            tmp.replace(out)
            return out
    except Exception:
        pass
    finally:
        tmp.unlink(missing_ok=True)
    return None


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
    img = rendered_still(cfg, item, output) or still_for(item)
    if not img:
        return
    cmd = _cmd(backend, img, output, cfg.data.get("palette_command", ""))
    if not cmd:
        return
    try:
        subprocess.run(cmd, capture_output=True, timeout=20)
    except Exception:
        pass
