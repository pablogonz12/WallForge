"""Optional niri integration: show the live wallpaper behind niri's overview.

Manages ``$XDG_CONFIG_HOME/niri/wallforge.kdl`` (a layer-rule with place-within-backdrop, or a comment-only file)
and makes sure niri's config.kdl includes it once. No-op when niri is not the compositor.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

FILE_NAME = "wallforge.kdl"
INCLUDE = f'include optional=true "{FILE_NAME}"'
BACKUP = "config.kdl.bak-wallforge"

_RULE = """layer-rule {
    match namespace="^wallforge$"
    place-within-backdrop true
}
"""


def render(on: bool, spawn: list[str] | None = None) -> str:
    """Content of the managed file: the overview-backdrop rule (when on) and the daemon's spawn-at-startup line."""
    head = ("// Managed by WallForge (Settings > Desktop > Fill the niri overview, Restore wallpapers at login). Do not edit.\n"
            if on or spawn else "// Managed by WallForge (Settings > Desktop > Fill the niri overview). Currently off.\n")
    body = ""
    if spawn:  # niri ignores XDG autostart; the daemon holds a lock, so a second copy exits at once
        body += "spawn-at-startup " + " ".join(json.dumps(a) for a in spawn) + "\n"
    return head + body + (_RULE if on else "")


def niri_dir() -> Path:
    return Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "niri"


def available() -> bool:
    """True when running under niri and its config folder exists."""
    on_niri = bool(os.environ.get("NIRI_SOCKET")) or "niri" in os.environ.get("XDG_CURRENT_DESKTOP", "").lower()
    return on_niri and (niri_dir() / "config.kdl").exists()


def _validate(cfg: Path) -> str:
    """Empty string when valid (or niri is not installed), else niri's error."""
    if not shutil.which("niri"):
        return ""
    try:
        r = subprocess.run(["niri", "validate", "-c", str(cfg)], capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.SubprocessError) as e:
        return str(e)
    return "" if r.returncode == 0 else (r.stderr or r.stdout).strip()


def apply(on: bool, spawn: list[str] | None = None) -> str:
    """Make the niri config match the setting. Returns "" on success or no-op, else an error message."""
    if not available():
        return ""
    d = niri_dir()
    main, mine = d / "config.kdl", d / FILE_NAME
    old_mine = mine.read_text() if mine.exists() else None
    old_main = main.read_text()
    new_mine = render(on, spawn)
    has_include = any(ln.strip() == INCLUDE for ln in old_main.splitlines())
    if old_mine == new_mine and has_include:
        return ""
    try:
        mine.write_text(new_mine)
        if not has_include:
            bak = d / BACKUP
            if not bak.exists():
                shutil.copy2(main, bak)
            sep = "" if old_main.endswith("\n") or not old_main else "\n"
            main.write_text(f"{old_main}{sep}\n// WallForge: live wallpaper in the overview (managed by WallForge)\n{INCLUDE}\n")
        err = _validate(main)
    except OSError as e:
        err = str(e)
    if err:
        if old_mine is None:
            mine.unlink(missing_ok=True)
        else:
            mine.write_text(old_mine)
        main.write_text(old_main)
        return err
    return ""
