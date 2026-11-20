"""`wallforge --doctor`: check this machine's setup and say what to fix.

Everything here is read-only and works without a display, so it is safe to run over SSH or paste into an issue.
"""

from __future__ import annotations

import json
import os
import platform
import shutil
import sys
from glob import glob
from pathlib import Path

from . import __version__, paths

OK, WARN, BAD = "ok", "warn", "fail"


def _gtk() -> tuple[str | None, str]:
    try:
        import gi

        gi.require_version("Gtk", "4.0")
        gi.require_version("Adw", "1")
        from gi.repository import Adw, Gtk

        return (
            f"{Gtk.get_major_version()}.{Gtk.get_minor_version()}.{Gtk.get_micro_version()}",
            f"{Adw.get_major_version()}.{Adw.get_minor_version()}",
        )
    except Exception as e:  # ImportError, ValueError from require_version, missing typelib
        return None, str(e)


def _typelib(name: str, version: str) -> bool:
    try:
        import gi

        gi.require_version(name, version)
        __import__("gi.repository", fromlist=[name])
        return True
    except Exception:
        return False


def _layer_shell() -> tuple[bool, str]:
    if _typelib("Gtk4LayerShell", "1.0"):
        return True, ""
    libs = glob("/usr/lib/libgtk4-layer-shell.so*") + glob("/usr/lib64/libgtk4-layer-shell.so*")
    if libs:
        return True, ""
    return False, "not found: web wallpapers and the wallpaper layer need gtk4-layer-shell"


def _session() -> tuple[str, str]:
    kind = (os.environ.get("XDG_SESSION_TYPE") or "").lower()
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", "") or "?"
    if not os.environ.get("WAYLAND_DISPLAY") and kind != "wayland":
        return BAD, f"{desktop} on {kind or 'an unknown session'}: WallForge draws with layer-shell, which needs Wayland"
    if "GNOME" in desktop.upper():
        return BAD, f"{desktop} does not implement layer-shell; use niri, Hyprland, Sway, river or Plasma 6.7+"
    return OK, f"{desktop} on {kind or 'wayland'}"


def _renderer() -> tuple[str, str]:
    rend = paths.find_render()
    if not rend:
        return BAD, "not built: run ./install.sh, or cargo build --release in render/"
    return OK, str(rend)


def _assets(cfg_assets: str) -> tuple[str, str]:
    a = paths.find_assets(cfg_assets)
    if not a:
        return BAD, "not found: Settings > Setup > Fetch, or install Wallpaper Engine in Steam"
    if not (a / "shaders").is_dir():
        return WARN, f"{a} has no shaders/, so scene wallpapers cannot render"
    return OK, str(a)


def _workshop() -> tuple[str, str]:
    d = paths.primary_workshop_dir()
    if not d or not d.is_dir():
        return WARN, "no Steam Workshop library; subscribe to wallpapers in Wallpaper Engine to get some"
    n = sum(1 for p in d.iterdir() if p.is_dir() and not p.name.startswith("."))
    if n == 0:
        return WARN, f"{d} is empty"
    return OK, f"{n} wallpapers in {d}"


def _video() -> tuple[str, str]:
    ff = shutil.which("ffmpeg")
    if not ff:
        return BAD, "ffmpeg not found: video wallpapers and thumbnails need it (install ffmpeg)"
    devs = glob("/dev/dri/renderD*")
    if not devs:
        return WARN, f"{ff}; no /dev/dri/renderD* device, so video decode stays on the CPU"
    return OK, f"{ff}, VAAPI device {devs[-1]}"


def _config() -> tuple[str, str, dict]:
    """(status, message, raw config). Read directly: Config.load() silently falls back to defaults."""
    from .config import CONFIG_FILE, DEFAULTS

    if not CONFIG_FILE.exists():
        return WARN, f"no config yet at {CONFIG_FILE} (it is written on first run)", {}
    try:
        raw = json.loads(CONFIG_FILE.read_text(encoding="utf-8"))
    except Exception as e:
        return BAD, f"{CONFIG_FILE} is not valid JSON ({e}); move it aside and start again", {}
    ver = raw.get("version", 1)
    want = DEFAULTS.get("version", 1)
    if ver < want:
        return WARN, f"v{ver} at {CONFIG_FILE}, written by an older WallForge (it migrates on start)", raw
    return OK, f"v{ver} at {CONFIG_FILE}", raw


def _previous_run() -> tuple[str, str, str] | None:
    """A renderer that died last time leaves a trace in its log; surface it instead of making people hunt."""
    d = Path(os.environ.get("XDG_RUNTIME_DIR") or "/tmp") / "wallforge"
    logs = sorted(d.glob("*.log")) if d.is_dir() else []
    if not logs:
        return None
    last = max(logs, key=lambda p: p.stat().st_mtime)
    try:
        tail = last.read_text(errors="replace").strip().splitlines()[-1]
    except Exception:
        return None
    if any(k in tail.lower() for k in ("panic", "error", "failed", "skipping", "not found")):
        return WARN, "last run", f"{last.name} ends with: {tail[:160]}"
    return OK, "last run", f"{last.name} ends cleanly"


def checks() -> list[tuple[str, str, str]]:
    """(status, label, detail or fix), in the order a setup problem usually appears."""
    out: list[tuple[str, str, str]] = []
    v = sys.version_info
    out.append(
        (OK if v >= (3, 11) else BAD, f"Python {platform.python_version()}",
         platform.platform() if v >= (3, 11) else "Python 3.11 or newer is required")
    )
    gtk, adw = _gtk()
    if gtk is None:
        out.append((BAD, "GTK 4 / libadwaita", f"not importable ({adw}); install python-gobject gtk4 libadwaita"))
    else:
        out.append((OK, f"GTK {gtk}", f"libadwaita {adw}"))
    ok, why = _layer_shell()
    out.append((OK if ok else BAD, "gtk4-layer-shell", why or "installed"))
    st, msg = _session()
    out.append((st, "session", msg))
    st, msg, raw = _config()
    out.append((st, "config", msg))
    st, msg = _renderer()
    out.append((st, "renderer", msg))
    st, msg = _assets(raw.get("assets_dir", ""))
    out.append((st, "Wallpaper Engine assets", msg))
    st, msg = _workshop()
    out.append((st, "Workshop library", msg))
    st, msg = _video()
    out.append((st, "video", msg))
    webok = _typelib("WebKit", "6.0")
    out.append((OK if webok else WARN, "WebKit 6.0",
                "installed" if webok else "web wallpapers are unavailable without webkitgtk-6.0"))
    prev = _previous_run()
    if prev:
        out.append(prev)
    return out


def report(entries: list[tuple[str, str, str]]) -> tuple[str, int]:
    """Format the checks. Returns the text to print and an exit code (0 unless something is broken)."""
    lines = [f"WallForge {__version__} on {platform.platform()}", ""]
    width = max((len(label) for _, label, _ in entries), default=0)
    icon = {OK: "ok  ", WARN: "warn", BAD: "FAIL"}
    for status, label, detail in entries:
        lines.append(f"  {icon.get(status, status)}  {label.ljust(width)}  {detail}".rstrip())
    bad = sum(1 for s, _, _ in entries if s == BAD)
    warn = sum(1 for s, _, _ in entries if s == WARN)
    lines.append("")
    if bad:
        lines.append(f"{bad} problem{'s' if bad > 1 else ''}, {warn} warning{'s' if warn != 1 else ''}. "
                     "Fix the FAIL lines first; details in docs/troubleshooting.md.")
    elif warn:
        lines.append(f"No problems. {warn} warning{'s' if warn > 1 else ''}; everything else is fine.")
    else:
        lines.append("No problems found.")
    return "\n".join(lines), (1 if bad else 0)


def main() -> int:
    text, code = report(checks())
    print(text)
    return code
