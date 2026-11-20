"""Install and enable WallForge's GNOME Shell extension (GNOME has no wlr-layer-shell, see gnome-extension/)."""
from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

UUID = "wallforge@pgm1207.github.io"


def on_gnome() -> bool:
    return "GNOME" in os.environ.get("XDG_CURRENT_DESKTOP", "").upper()


def source() -> Path | None:
    here = Path(__file__).resolve().parent
    for d in (here.parent / "gnome-extension" / UUID, Path("/usr/share/gnome-shell/extensions") / UUID):
        if (d / "metadata.json").is_file():
            return d
    return None


def target() -> Path:
    return Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "gnome-shell/extensions" / UUID


def installed() -> bool:
    return (target() / "metadata.json").is_file() or (Path("/usr/share/gnome-shell/extensions") / UUID / "metadata.json").is_file()


def install() -> tuple[bool, str]:
    """Copy the extension to the user's extensions folder and enable it. Returns (ok, message)."""
    src = source()
    if not src:
        return False, "The extension files were not found."
    dst = target()
    if src.resolve() != dst.resolve():
        shutil.rmtree(dst, ignore_errors=True)
        shutil.copytree(src, dst, ignore=shutil.ignore_patterns("__pycache__"))
    if not shutil.which("gnome-extensions"):
        return True, f"Copied to {dst}. Enable it with: gnome-extensions enable {UUID}. Log out and back in first."
    r = subprocess.run(["gnome-extensions", "enable", UUID], capture_output=True, text=True, timeout=20)
    if r.returncode != 0:
        return True, (f"Copied to {dst}. GNOME does not know it yet: log out and back in, then run: gnome-extensions enable {UUID}")
    return True, "Installed and enabled. On Wayland, GNOME only finds new extensions after you log out and back in (first time only)."
