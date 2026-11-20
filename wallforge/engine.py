"""Launch and stop the wallpaper player, one process per output: wallforge-render for videos and scenes, the WebKit
player for web wallpapers, linux-wallpaperengine as a fallback when it is installed."""
from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from . import library, paths, texpatch, webplayer

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


_web_missing: str | None = None


def web_missing() -> str:
    """Empty when web wallpapers can play, else the packages that are missing."""
    global _web_missing
    if _web_missing is None:
        try:
            _web_missing = webplayer.available()
        except Exception as e:
            _web_missing = str(e) or "PyGObject"
    return _web_missing


def use_web(item: library.Item) -> bool:
    return item.kind == "web" and not item.broken and not web_missing()


def build_web_args(cfg, item: library.Item, output: str, overrides: dict | None = None) -> list[str]:
    """Command line of the WebKit player (wallforge/webplayer.py) for a web wallpaper."""
    e = cfg.data["engine"]
    props = item.properties()
    vals = dict(item.defaults())
    vals.update(cfg.values_for(item.id))
    vals.update(overrides or {})
    fps, _scale = effective(cfg)
    a = [sys.executable, "-m", "wallforge.webplayer", "--output", output, "--bg", str(item.run_dir),
         "--fps", str(fps), "--layer", e.get("layer", "background")]
    vol = cfg.volume_for(item.id)
    a += ["--volume", str(vol)] if vol else ["--silent"]
    if not e.get("disable_mouse", True):
        a.append("--mouse")
    for k, v in vals.items():
        spec = props.get(k)
        if isinstance(spec, dict) and v not in (None, ""):
            a += ["--set-property", f"{k}={_fmt(spec, v)}"]
    return a


def use_native(cfg, item: library.Item) -> bool:
    """Native renderer is on, installed, and the wallpaper is a video or a scene (scenes need the assets)."""
    if cfg.data.get("renderer") != "native" or item.broken or paths.find_render() is None:
        return False
    if item.kind == "video":
        return True
    return item.kind == "scene" and paths.find_assets(cfg.data["assets_dir"]) is not None


_ASPECT_CACHE: dict[str, float | None] = {}


def on_battery() -> bool:
    """True when a real battery reports Discharging (fake power supplies whose type is not Battery are ignored)."""
    try:
        for d in Path("/sys/class/power_supply").iterdir():
            try:
                if (d / "type").read_text().strip().lower() != "battery":
                    continue
                if (d / "status").read_text().strip().lower() == "discharging":
                    return True
            except Exception:
                continue
    except Exception:
        pass
    return False


def effective(cfg) -> tuple[int, int]:
    """(fps, render_scale) to use now, lowered when on battery and the setting allows it."""
    e = cfg.data["engine"]
    try:
        fps = max(1, int(e.get("fps", 30)))
    except (TypeError, ValueError):
        fps = 30
    try:
        scale = max(25, min(100, int(e.get("render_scale", 100))))
    except (TypeError, ValueError):
        scale = 100
    if e.get("reduce_on_battery", True) and on_battery():
        fps = max(15, min(fps, 30))
        scale = min(scale, 75)
    return fps, scale


def pause_for_battery(cfg) -> bool:
    """True when wallpapers should be paused because of the battery setting."""
    return bool(cfg.data["engine"].get("pause_on_battery")) and on_battery()



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


def window_mode() -> bool:
    """True where the compositor has no wlr-layer-shell (GNOME): the renderer then runs as a window for the extension."""
    if os.environ.get("WALLFORGE_WINDOW") in ("0", "1"):
        return os.environ["WALLFORGE_WINDOW"] == "1"
    return "GNOME" in os.environ.get("XDG_CURRENT_DESKTOP", "").upper()


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
    fps, scale = effective(cfg)
    a = [str(paths.find_render()), "--output", output, "--bg", str(run), "--scaling", scaling,
         "--fps", str(fps), "--layer", e.get("layer", "background")]
    if window_mode():
        a.append("--window")
    vol = cfg.volume_for(item.id)
    a += ["--volume", str(vol)] if vol else ["--silent"]
    if item.kind == "scene":
        a += ["--scale", str(scale)]
        a_dir = paths.find_assets(cfg.data["assets_dir"])
        if a_dir:
            a += ["--assets-dir", str(a_dir)]
        if e.get("disable_particles"):
            a.append("--disable-particles")
        if e.get("no_audio_processing"):
            a.append("--no-audio-processing")
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
         "--layer", e.get("layer", "background")]
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


def _spare_file(output: str): return STATE / f"{output}.spare"


def _layer_file(output: str): return STATE / f"{output}.layer"


def current_layer(output: str) -> str:
    """The layer the visible renderer actually sits on ("bottom" or "background")."""
    try:
        return _layer_file(output).read_text().strip() or "bottom"
    except Exception:
        return "bottom"


def spare(output: str) -> dict | None:
    """The preloaded-but-hidden renderer for this output: {"id", "pid"}, if it is still alive."""
    try:
        d = json.loads(_spare_file(output).read_text())
        return d if Path(f"/proc/{int(d['pid'])}").exists() else None
    except Exception:
        return None


def find_pids(output: str) -> list[int]:
    """Every engine process serving this output (ours or another launcher's), by /proc scan."""
    hide = {int((spare(output) or {}).get("pid", -1))}
    res = []
    for p in Path("/proc").glob("[0-9]*"):
        try:
            comm = (p / "comm").read_text()
            if not comm.startswith(("linux-wallpaper", "wallforge-rend", "wallforge-web")):
                continue
            args = (p / "cmdline").read_bytes().split(b"\0")
            for i, a in enumerate(args[:-1]):
                if a in (b"--screen-root", b"-r", b"--output") and args[i + 1].decode(errors="replace") == output:
                    if int(p.name) not in hide:
                        res.append(int(p.name))
                    break
        except Exception:
            continue
    return res


def preload(cfg, item: library.Item, output: str) -> bool:
    """Load the wallpaper that will probably be next, on the layer below, so switching to it is one frame.

    The spare runs `--hidden` on the same layer as the visible wallpaper: it loads and renders but never presents until
    promote() signals it, so it is invisible (also to niri's blur, which samples the background layer). Costs the
    spare's VRAM until it is used.
    """
    # The spare lives on the SAME layer as the visible wallpaper (niri's blur/xray samples only `background`, so the
    # visible one must be there if the user chose it) and is started `--hidden`: it never presents until promoted.
    if window_mode():
        return False  # window mode (GNOME) has no layers
    if item.kind == "web" or not use_native(cfg, item):
        return False
    cur = spare(output)
    if cur and cur.get("id") == item.id:
        return True
    stop_spare(output)
    ready = STATE / f"{output}.spare.ready"
    ready.unlink(missing_ok=True)
    args = build_native_args(cfg, item, output)
    args += ["--hidden", "--ready-file", str(ready)]
    STATE.mkdir(parents=True, exist_ok=True)
    log = open(STATE / f"{output}.spare.log", "w")
    try:
        p = subprocess.Popen(args, cwd=str(Path(args[0]).parent), env=dict(os.environ), stdout=log,
                             stderr=subprocess.STDOUT, start_new_session=True)
    except Exception:
        return False
    _spare_file(output).write_text(json.dumps({"id": item.id, "pid": p.pid, "ready": str(ready)}))
    return True


def stop_spare_all():
    """Drop every preloaded wallpaper (the setting was turned off, or we are quitting)."""
    for f in STATE.glob("*.spare"):
        try:
            os.kill(int(json.loads(f.read_text())["pid"]), signal.SIGTERM)
        except Exception:
            pass
        f.unlink(missing_ok=True)


def stop_spare(output: str):
    sp = spare(output)
    if sp:
        try: os.kill(int(sp["pid"]), signal.SIGTERM)
        except ProcessLookupError: pass
    _spare_file(output).unlink(missing_ok=True)


def promote(cfg, item: library.Item, output: str) -> bool:
    """If the wanted wallpaper is already preloaded, retire the current renderer and make the spare the current one."""
    sp = spare(output)
    if not sp or sp.get("id") != item.id:
        return False
    pids = find_pids(output)
    try: os.kill(int(sp["pid"]), signal.SIGUSR1)  # start presenting (it maps above the old one, same layer)
    except ProcessLookupError: return False
    for pid in pids:
        try: os.kill(pid, signal.SIGTERM)
        except ProcessLookupError: pass
    _pidfile(output).write_text(str(sp["pid"]))
    _spare_file(output).unlink(missing_ok=True)
    return True


def running(output: str) -> int | None:
    pids = find_pids(output)
    return pids[0] if pids else None


def stop(output: str, wait: float = 3.0):
    stop_spare(output)
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
    # Switching should look like a cut, not a black frame. Instead of killing the running wallpaper first, start the
    # new one, wait for its first presented frame (the renderer touches --ready-file), and only then retire the old
    # process. Web wallpapers are the exception: two WebKit layers on one output fight, so they stop first.
    if not legacy and promote(cfg, item, output):
        return int(_pidfile(output).read_text())
    prev = [] if legacy else [p for p in find_pids(output) if Path(f"/proc/{p}").exists()]
    web_item = use_web(item) and not legacy
    if not prev or web_item:
        stop(output)
    ready = STATE / f"{output}.ready"
    ready.unlink(missing_ok=True)
    STATE.mkdir(parents=True, exist_ok=True)
    web = use_web(item) and not legacy
    native = not web and use_native(cfg, item) and not legacy
    _marker(output).write_text("web" if web else "native" if native else "legacy")
    log = open(STATE / f"{output}.log", "w")
    if web:
        args = build_web_args(cfg, item, output, overrides)
        pkg_root = str(Path(__file__).resolve().parent.parent)
        env = dict(os.environ)
        env["PYTHONPATH"] = pkg_root + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        p = subprocess.Popen(args, cwd=pkg_root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        _pidfile(output).write_text(str(p.pid))
        return p.pid
    args = build_native_args(cfg, item, output, overrides) if native else build_args(cfg, item, output, overrides)
    if native:
        args += ["--ready-file", str(ready)]
    eng_dir = Path(args[0]).parent
    p = subprocess.Popen(args, cwd=str(eng_dir), env=dict(os.environ) if native else _env(eng_dir), stdout=log, stderr=subprocess.STDOUT,
                         start_new_session=True)
    _pidfile(output).write_text(str(p.pid))
    _layer_file(output).write_text("background" if native and "--layer" in args and args[args.index("--layer") + 1] == "background" else "bottom")
    if prev and native:
        end = time.monotonic() + 2.0
        while time.monotonic() < end and not ready.exists() and p.poll() is None:
            time.sleep(0.02)
        for q in prev:
            try: os.kill(q, signal.SIGTERM)
            except ProcessLookupError: pass
    return p.pid


def log_tail(output: str, n=40) -> str:
    try:
        return "".join((STATE / f"{output}.log").read_text(errors="replace").splitlines(True)[-n:])
    except Exception:
        return ""
