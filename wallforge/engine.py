"""Launch and stop the wallpaper renderer (native wallforge-render for videos, else linux-wallpaperengine), one process per output."""
from __future__ import annotations

import json
import os
import signal
import subprocess
import time
from pathlib import Path

from . import library, paths, texpatch

STATE = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "wallforge"


def _fmt(spec: dict, val) -> str:
    t = (spec.get("type") or "").lower()
    if t == "bool":
        return "true" if val in (True, 1, "1", "true", "True") else "false"
    if t == "color":
        if isinstance(val, (list, tuple)):
            return " ".join(str(v) for v in val)
        return str(val)
    return str(val)


def use_native(cfg, item: library.Item) -> bool:
    """Native renderer is on, installed, and the wallpaper is a video or a scene (scenes need the assets)."""
    if cfg.data.get("renderer") != "native" or item.broken or paths.find_render() is None:
        return False
    if item.kind == "video":
        return True
    return item.kind == "scene" and paths.find_assets(cfg.data["assets_dir"]) is not None


_ASPECT_CACHE: dict[str, float | None] = {}


def wallpaper_aspect(item: library.Item) -> float | None:
    """The wallpaper's own width/height, from the scene projection or by probing the video."""
    if item.aspect:
        return item.aspect
    key = str(item.run_dir)
    if key not in _ASPECT_CACHE:
        asp = None
        if item.kind == "video" and paths.find_render():
            try:
                r = subprocess.run([str(paths.find_render()), "--check", key], capture_output=True, text=True, timeout=10)
                j = json.loads(r.stdout.strip().splitlines()[-1])
                if j.get("ok") and j.get("width") and j.get("height"):
                    asp = j["width"] / j["height"]
            except Exception:
                pass
        _ASPECT_CACHE[key] = asp
    return _ASPECT_CACHE[key]


def best_scaling(item: library.Item, output: str) -> str:
    """Auto mode: fill (crop to cover) unless the picture is much narrower than the screen (portrait), which fit keeps whole."""
    from . import monitors
    try:
        mon = next((m for m in monitors.outputs() if m["name"] == output), None)
        wp = wallpaper_aspect(item)
        if not mon or not wp or not mon.get("height"):
            return "fill"
        r = wp / (mon["width"] / mon["height"])
        return "fill" if r >= 0.7 else "fit"
    except Exception:
        return "fill"


def build_native_args(cfg, item: library.Item, output: str, overrides: dict | None = None) -> list[str]:
    e = cfg.data["engine"]
    scaling = e["scaling"]
    if scaling == "auto":
        scaling = best_scaling(item, output)
    elif scaling not in ("fill", "fit", "stretch"):
        scaling = "fill"
    props = item.properties()
    vals = dict(item.defaults())
    vals.update(cfg.values_for(item.id))
    vals.update(overrides or {})
    run = item.run_dir
    if item.kind == "scene":
        imgs = {k: v for k, v in vals.items() if (props.get(k) or {}).get("type") == "scenetexture"}
        if imgs:
            run = texpatch.patched_dir(item.run_dir, imgs) or item.run_dir  # None when there is no package to patch
    a = [str(paths.find_render()), "--output", output, "--bg", str(run), "--scaling", scaling,
         "--fps", str(e["fps"]), "--layer", e.get("layer", "bottom")]
    vol = cfg.volume_for(item.id)
    a += ["--volume", str(vol)] if vol else ["--silent"]
    if item.kind == "scene":
        a_dir = paths.find_assets(cfg.data["assets_dir"])
        if a_dir:
            a += ["--assets-dir", str(a_dir)]
        for k, v in vals.items():
            spec = props.get(k)
            if isinstance(spec, dict) and v not in (None, "") and spec.get("type") != "scenetexture":
                a += ["--set-property", f"{k}={_fmt(spec, v)}"]
    return a


def build_args(cfg, item: library.Item, output: str, overrides: dict | None = None) -> list[str]:
    e = cfg.data["engine"]
    eng = paths.find_engine(cfg.data["engine_dir"])
    if not eng:
        raise RuntimeError("linux-wallpaperengine not found")
    a_dir = paths.find_assets(cfg.data["assets_dir"])
    if not a_dir:
        raise RuntimeError("Wallpaper Engine assets not found (see Settings > Setup)")
    assets = str(a_dir)
    props = item.properties()
    vals = dict(item.defaults())  # base <- preset
    vals.update(cfg.values_for(item.id))  # <- user
    vals.update(overrides or {})
    # engine ignores image properties: bake them into a patched copy of the scene package
    imgs = {k: v for k, v in vals.items() if (props.get(k) or {}).get("type") == "scenetexture"}
    run = texpatch.patched_dir(item.run_dir, imgs) if imgs else None
    a = [str(eng), "--screen-root", output, "--bg", str(run or item.run_dir), "--assets-dir", assets,
         "--scaling", ("fill" if e["scaling"] == "auto" else e["scaling"]), "--clamp", e["clamp"], "--fps", str(e["fps"]),
         "--layer", e.get("layer", "bottom")]
    vol = cfg.volume_for(item.id)
    a += ["--volume", str(vol)] if vol else ["--silent"]
    for flag, key in (("--no-audio-processing", "no_audio_processing"), ("--disable-particles", "disable_particles"),
                      ("--disable-mouse", "disable_mouse"), ("--disable-parallax", "disable_parallax"),
                      ("--no-fullscreen-pause", "no_fullscreen_pause")):
        if e.get(key):
            a.append(flag)
    if e.get("fullscreen_pause_only_active") and not e.get("no_fullscreen_pause"):
        a.append("--fullscreen-pause-only-active")
    for k, v in vals.items():
        spec = props.get(k)
        if not isinstance(spec, dict) or v in (None, "") or spec.get("type") == "scenetexture":
            continue
        a += ["--set-property", f"{k}={_fmt(spec, v)}"]
    return a


def _env(eng_dir: Path):
    env = dict(os.environ)
    extra = [str(p) for p in (eng_dir / "lib", eng_dir.parent / "lwe-deps/usr/lib") if p.is_dir()]
    env["LD_LIBRARY_PATH"] = ":".join(filter(None, extra + [env.get("LD_LIBRARY_PATH", "")]))
    return env


def _pidfile(output): return STATE / f"{output}.pid"
def _marker(output): return STATE / f"{output}.kind"


def was_native(output: str) -> bool:
    try:
        return _marker(output).read_text() == "native"
    except Exception:
        return False


def find_pids(output: str) -> list[int]:
    """Every engine process serving this output (ours or another launcher's), by /proc scan."""
    res = []
    for p in Path("/proc").glob("[0-9]*"):
        try:
            comm = (p / "comm").read_text()
            if not comm.startswith(("linux-wallpaper", "wallforge-rend")):
                continue
            args = (p / "cmdline").read_bytes().split(b"\0")
            for i, a in enumerate(args[:-1]):
                if a in (b"--screen-root", b"-r", b"--output") and args[i + 1].decode(errors="replace") == output:
                    res.append(int(p.name))
                    break
        except Exception:
            continue
    return res


def running(output: str) -> int | None:
    pids = find_pids(output)
    return pids[0] if pids else None


def stop(output: str, wait: float = 3.0):
    pids = find_pids(output)
    for pid in pids:
        try: os.kill(pid, signal.SIGTERM)
        except ProcessLookupError: pass
    end = time.time() + wait
    while time.time() < end and any(Path(f"/proc/{p}").exists() for p in pids):
        time.sleep(0.1)
    for pid in pids:
        if Path(f"/proc/{pid}").exists():
            try: os.kill(pid, signal.SIGKILL)
            except ProcessLookupError: pass
    _pidfile(output).unlink(missing_ok=True)


def start(cfg, item: library.Item, output: str, overrides: dict | None = None, legacy: bool = False) -> int:
    stop(output)
    STATE.mkdir(parents=True, exist_ok=True)
    native = use_native(cfg, item) and not legacy
    _marker(output).write_text("native" if native else "legacy")
    args = build_native_args(cfg, item, output, overrides) if native else build_args(cfg, item, output, overrides)
    log = open(STATE / f"{output}.log", "w")
    eng_dir = Path(args[0]).parent
    p = subprocess.Popen(args, cwd=str(eng_dir), env=dict(os.environ) if native else _env(eng_dir), stdout=log, stderr=subprocess.STDOUT,
                         start_new_session=True)
    _pidfile(output).write_text(str(p.pid))
    return p.pid


def log_tail(output: str, n=40) -> str:
    try:
        return "".join((STATE / f"{output}.log").read_text(errors="replace").splitlines(True)[-n:])
    except Exception:
        return ""
