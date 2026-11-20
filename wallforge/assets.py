"""Get Wallpaper Engine's `assets/` folder without installing the whole game.

The assets are Wallpaper Engine's own files and are never bundled or redistributed. Two ways to get them,
both of which need the Steam account to own Wallpaper Engine (appid 431960):

1. DepotDownloader (open source, SteamRE) fetches only `assets/` (~85 MB) from Steam's servers. The Steam
   login happens inside DepotDownloader in a terminal window; wallforge never sees the password.
2. Fallback: ask the running Steam client to install the app (steam://install/431960), then copy `assets/`.
"""
from __future__ import annotations

import hashlib
import os
import random
import shlex
import shutil
import stat
import subprocess
import urllib.request
import zipfile
from pathlib import Path

from . import paths

APPID, DEPOT = "431960", "431961"
TOOLS = paths.DATA / "wallforge/tools"
DD_VERSION = "3.4.0"
DD_URL = f"https://github.com/SteamRE/DepotDownloader/releases/download/DepotDownloader_{DD_VERSION}/DepotDownloader-linux-x64.zip"
DD_SHA256 = "a999dec66b4850fc961bd50366696d23c2d0fad7b18790e6a5647b2f19097a53"
SCRIPT = paths.DATA / "wallforge/fetch-assets.sh"
STAGE = paths.DATA / "wallforge/we-dl"
TERMINALS = (("alacritty", "-e"), ("kitty", ""), ("foot", ""), ("ghostty", "-e"), ("wezterm", "start --"),
             ("konsole", "-e"), ("gnome-terminal", "--"), ("kgx", "-e"), ("xfce4-terminal", "-x"), ("xterm", "-e"))


def have_assets(configured: str = "") -> bool:
    return paths.find_assets(configured) is not None


def depot_downloader() -> Path | None:
    for name in ("DepotDownloader", "depotdownloader"):
        w = shutil.which(name)
        if w:
            return Path(w)
    p = TOOLS / "DepotDownloader"
    return p if p.is_file() and os.access(p, os.X_OK) else None


def install_depot_downloader(progress=None) -> Path:
    """Download the pinned DepotDownloader release, verify its SHA-256, unpack it into our own tools dir."""
    TOOLS.mkdir(parents=True, exist_ok=True)
    zpath = TOOLS / "dd.zip"
    h = hashlib.sha256()
    req = urllib.request.Request(DD_URL, headers={"User-Agent": "wallforge"})
    with urllib.request.urlopen(req, timeout=60) as r, open(zpath, "wb") as out:
        total = int(r.headers.get("Content-Length") or 0)
        done = 0
        while chunk := r.read(1 << 16):
            out.write(chunk)
            h.update(chunk)
            done += len(chunk)
            if progress and total:
                progress(done / total)
    if h.hexdigest() != DD_SHA256:
        zpath.unlink(missing_ok=True)
        raise RuntimeError("DepotDownloader download failed its checksum; refusing to run it")
    with zipfile.ZipFile(zpath) as z:
        z.extract("DepotDownloader", TOOLS)
    zpath.unlink(missing_ok=True)
    exe = TOOLS / "DepotDownloader"
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    return exe


def terminal_cmd() -> list[str] | None:
    pref = os.environ.get("TERMINAL")
    cands = ([(pref, "-e")] if pref else []) + list(TERMINALS)
    for exe, flag in cands:
        if exe and shutil.which(exe):
            return [exe] + shlex.split(flag)
    return None


def write_script(dd: Path) -> Path:
    """Shell script run in a terminal: logs in to Steam (prompts there), fetches assets/, moves them into place."""
    SCRIPT.parent.mkdir(parents=True, exist_ok=True)
    flist = paths.DATA / "wallforge/we-filelist.txt"
    flist.write_text("regex:^assets/\n")
    login_id = random.randint(10_000, 2_000_000_000)
    SCRIPT.write_text(f"""#!/bin/bash
echo "WallForge: fetching Wallpaper Engine's assets from Steam (about 85 MB)."
echo "You need an account that owns Wallpaper Engine. Your password goes to DepotDownloader only."
echo
read -r -p "Steam username: " U
[ -z "$U" ] && exit 1
rm -rf {shlex.quote(str(STAGE))}
{shlex.quote(str(dd))} -app {APPID} -depot {DEPOT} -os windows -username "$U" -remember-password \\
  -loginid {login_id} -filelist {shlex.quote(str(flist))} -dir {shlex.quote(str(STAGE))}
if [ -d {shlex.quote(str(STAGE))}/assets ]; then
  mkdir -p {shlex.quote(str(paths.OWN_ASSETS.parent))}
  rm -rf {shlex.quote(str(paths.OWN_ASSETS))}
  mv {shlex.quote(str(STAGE))}/assets {shlex.quote(str(paths.OWN_ASSETS))}
  rm -rf {shlex.quote(str(STAGE))}
  echo; echo "Done. You can close this window and return to WallForge."
else
  echo; echo "No assets were downloaded. If Steam said access denied, this account does not own Wallpaper Engine."
fi
read -r -p "Press Enter to close. " _
""")
    SCRIPT.chmod(0o700)
    return SCRIPT


def launch_download() -> str:
    """Open the terminal that does the fetch. Returns '' on success or a human-readable problem."""
    dd = depot_downloader()
    if not dd:
        return "DepotDownloader is missing"
    term = terminal_cmd()
    if not term:
        return "No terminal emulator found. Run this yourself: bash " + str(write_script(dd))
    script = write_script(dd)
    subprocess.Popen(term + ["bash", str(script)], start_new_session=True)
    return ""


def steam_install_uri() -> str:
    return f"steam://install/{APPID}"


def copy_from_steam_install() -> bool:
    """After the Steam client installed Wallpaper Engine, copy its assets into our own folder."""
    for d in paths.we_dirs():
        src = d / "assets"
        if (src / "shaders").is_dir():
            shutil.copytree(src, paths.OWN_ASSETS, dirs_exist_ok=True)
            return True
    return False
