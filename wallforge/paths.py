"""Find Steam, Workshop content and Wallpaper Engine assets on any common Linux layout."""
from __future__ import annotations

import os
import re
import shutil
from pathlib import Path

APPID = "431960"
HOME = Path.home()
DATA = Path(os.environ.get("XDG_DATA_HOME", HOME / ".local/share"))
LOCAL_LIBRARY = DATA / "wallforge/library"
OWN_ASSETS = DATA / "wallforge/assets"  # assets fetched by wallforge itself


def steam_roots() -> list[Path]:
    cands = [os.environ.get("WALLFORGE_STEAM"), DATA / "Steam", HOME / ".steam/steam", HOME / ".steam/root",
             HOME / ".var/app/com.valvesoftware.Steam/.local/share/Steam",
             HOME / ".var/app/com.valvesoftware.Steam/data/Steam", HOME / "snap/steam/common/.local/share/Steam"]
    seen, out = set(), []
    for c in cands:
        if not c:
            continue
        p = Path(c)
        try:
            real = p.resolve()
        except Exception:
            continue
        if real in seen or not (real / "steamapps").is_dir():
            continue
        seen.add(real)
        out.append(real)
    return out


def libraries() -> list[Path]:
    """Every Steam library folder (the main one plus those listed in libraryfolders.vdf)."""
    libs: list[Path] = []
    for root in steam_roots():
        libs.append(root)
        vdf = root / "steamapps/libraryfolders.vdf"
        try:
            for m in re.finditer(r'"path"\s+"([^"]+)"', vdf.read_text(errors="replace")):
                p = Path(m.group(1))
                if (p / "steamapps").is_dir():
                    libs.append(p.resolve())
        except Exception:
            pass
    seen, out = set(), []
    for l in libs:
        if l not in seen:
            seen.add(l)
            out.append(l)
    return out


def workshop_dirs() -> list[Path]:
    return [l / f"steamapps/workshop/content/{APPID}" for l in libraries() if (l / f"steamapps/workshop/content/{APPID}").is_dir()]


def primary_workshop_dir() -> Path:
    """Where new Workshop downloads land (first library with the folder, else the main Steam dir)."""
    d = workshop_dirs()
    if d:
        return d[0]
    roots = steam_roots()
    return (roots[0] if roots else DATA / "Steam") / f"steamapps/workshop/content/{APPID}"


def we_dirs() -> list[Path]:
    return [l / "steamapps/common/wallpaper_engine" for l in libraries() if (l / "steamapps/common/wallpaper_engine").is_dir()]


def find_assets(configured: str = "") -> Path | None:
    cands = [Path(configured)] if configured else []
    cands += [OWN_ASSETS] + [d / "assets" for d in we_dirs()]
    for c in cands:
        if (c / "shaders").is_dir() or (c / "materials").is_dir():
            return c
    return None


def find_engine(configured: str = "") -> Path | None:
    """Locate the linux-wallpaperengine binary: configured dir, ~/.local/opt, PATH, system."""
    cands = []
    if configured:
        cands.append(Path(configured) / "linux-wallpaperengine")
    cands.append(HOME / ".local/opt/linux-wallpaperengine/linux-wallpaperengine")
    for name in ("linux-wallpaperengine",):
        w = shutil.which(name)
        if w:
            cands.append(Path(w))
    cands += [Path("/usr/bin/linux-wallpaperengine"), Path("/usr/local/bin/linux-wallpaperengine"),
              Path("/opt/linux-wallpaperengine/linux-wallpaperengine"),
              HOME / ".var/app/com.valvesoftware.Steam/.local/bin/linux-wallpaperengine"]
    for c in cands:
        try:
            if c.is_file() and os.access(c, os.X_OK):
                return c.resolve() if c.is_symlink() and "wallforge" not in str(c) else c
        except Exception:
            pass
    return None


def find_render(configured: str = "") -> Path | None:
    """Locate the native wallforge-render binary: source tree build, system install, PATH."""
    here = Path(__file__).resolve().parent
    cands = [Path(configured) / "wallforge-render"] if configured else []
    cands += [here.parent / "render/target/release/wallforge-render", here.parent / "wallforge-render",
              Path("/usr/lib/wallforge/wallforge-render")]
    w = shutil.which("wallforge-render")
    if w:
        cands.append(Path(w))
    for c in cands:
        try:
            if c.is_file() and os.access(c, os.X_OK):
                return c
        except Exception:
            pass
    return None


def render_source() -> Path | None:
    """The render/ crate when running from a git checkout (so it can be built in place)."""
    d = Path(__file__).resolve().parent.parent / "render"
    return d if (d / "Cargo.toml").is_file() else None


def local_dir() -> Path:
    """User library of wallpapers added from outside the Workshop (WALLFORGE_LIBRARY overrides, for tests)."""
    env = os.environ.get("WALLFORGE_LIBRARY")
    return Path(env) if env else LOCAL_LIBRARY
