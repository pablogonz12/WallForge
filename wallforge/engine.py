"""Launch and stop linux-wallpaperengine, one process per output."""
from __future__ import annotations

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
         "--scaling", e["scaling"], "--clamp", e["clamp"], "--fps", str(e["fps"]),
         "--layer", e.get("layer", "bottom")]
    if e.get("silent"):
        a.append("--silent")
    else:
        a += ["--volume", str(e["volume"])]
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


def find_pids(output: str) -> list[int]:
    """Every engine process serving this output (ours or another launcher's), by /proc scan."""
    res = []
    for p in Path("/proc").glob("[0-9]*"):
        try:
            if not (p / "comm").read_text().startswith("linux-wallpaper"):
                continue
            args = (p / "cmdline").read_bytes().split(b"\0")
            for i, a in enumerate(args[:-1]):
                if a in (b"--screen-root", b"-r") and args[i + 1].decode(errors="replace") == output:
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


def start(cfg, item: library.Item, output: str, overrides: dict | None = None) -> int:
    stop(output)
    STATE.mkdir(parents=True, exist_ok=True)
    args = build_args(cfg, item, output, overrides)
    log = open(STATE / f"{output}.log", "w")
    eng_dir = Path(args[0]).parent
    p = subprocess.Popen(args, cwd=str(eng_dir), env=_env(eng_dir), stdout=log, stderr=subprocess.STDOUT,
                         start_new_session=True)
    _pidfile(output).write_text(str(p.pid))
    return p.pid


def log_tail(output: str, n=40) -> str:
    try:
        return "".join((STATE / f"{output}.log").read_text(errors="replace").splitlines(True)[-n:])
    except Exception:
        return ""
