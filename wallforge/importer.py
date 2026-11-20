"""Import wallpapers that do not come from the Workshop into the user library.

Every import becomes a folder ``<local_dir>/local-<slug>/`` with a ``project.json`` in the same schema
the Workshop uses, so scan, apply, rotation and the renderer need no special cases.

Videos are *referenced* (a symlink to the original file), not copied: they are large and a copy
would double the disk use. Move or delete the original and the item shows as broken. Pass
``copy=True`` to copy instead. Web folders and Lively packages are copied/extracted.
"""
from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path, PurePosixPath

from . import paths

VIDEO_EXT = {".mp4", ".webm", ".mkv", ".mov", ".gif", ".m4v", ".avi"}
IMAGE_EXT = {".png", ".jpg", ".jpeg", ".webp", ".bmp"}
MAX_UNPACKED = 2 * 1024 ** 3  # zip bomb cap, total bytes
MAX_FILES = 20000

# Lively WallpaperType enum (Lively.Models/Enums/WallpaperType.cs), by number and by name.
_LIVELY_NUM = {0: "app", 1: "web", 2: "webaudio", 3: "url", 4: "bizhawk", 5: "unity", 6: "godot",
               7: "video", 8: "gif", 9: "unityaudio", 10: "videostream", 100: "picture"}
_UNSUPPORTED = {"app": "application", "bizhawk": "BizHawk emulator", "unity": "Unity", "unityaudio": "Unity",
                "godot": "Godot", "url": "web URL", "videostream": "video stream"}


class ImportError_(Exception):
    """Import refused; the message is meant for the user."""


def lively_type(value) -> str:
    """Normalise LivelyInfo 'Type' (string name or number) to a lowercase name."""
    if isinstance(value, bool):
        return ""
    if isinstance(value, int):
        return _LIVELY_NUM.get(value, "")
    s = str(value).strip().lower()
    if s.isdigit():
        return _LIVELY_NUM.get(int(s), "")
    return s


def _slug(name: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:40]
    return s or "item"


def _new_dir(name: str) -> Path:
    root = paths.local_dir()
    root.mkdir(parents=True, exist_ok=True)
    base = "local-" + _slug(name)
    d, n = root / base, 1
    while d.exists() or d.is_symlink():
        n += 1
        d = root / f"{base}-{n}"
    d.mkdir()
    return d


def _write_project(d: Path, data: dict) -> None:
    (d / "project.json").write_text(json.dumps(data, indent=2, ensure_ascii=False))


def _thumb(src: Path, dest: Path, video: bool = True) -> str | None:
    """Grab one frame (or scale an image) with ffmpeg; None when ffmpeg is missing or fails."""
    ff = shutil.which("ffmpeg")
    if not ff:
        return None
    cmd = [ff, "-v", "error", "-y"]
    if video:
        cmd += ["-ss", "1"]
    cmd += ["-i", str(src), "-frames:v", "1", "-vf", "scale=640:-2", str(dest)]
    try:
        subprocess.run(cmd, capture_output=True, timeout=30)
        if not dest.is_file() and video:  # clip shorter than 1 s
            subprocess.run([c for c in cmd if c not in ("-ss", "1")], capture_output=True, timeout=30)
        return dest.name if dest.is_file() else None
    except Exception:
        return None


def _meta(d: Path, data: dict, preview: str | None) -> dict:
    if preview:
        data["preview"] = preview
    return data


def import_video(path: Path, copy: bool = False) -> Path:
    path = path.resolve()
    d = _new_dir(path.stem)
    try:
        name = "video" + path.suffix.lower()
        if copy:
            shutil.copy2(path, d / name)
        else:
            (d / name).symlink_to(path)
        prev = _thumb(path, d / "preview.jpg")
        _write_project(d, _meta(d, {"type": "video", "file": name, "title": path.stem, "source": "Local"}, prev))
    except Exception:
        shutil.rmtree(d, ignore_errors=True)
        raise
    return d


def import_image(path: Path, copy: bool = False) -> Path:
    """A still image becomes a web wallpaper that shows it covering the screen."""
    path = path.resolve()
    d = _new_dir(path.stem)
    try:
        name = "image" + path.suffix.lower()
        if copy:
            shutil.copy2(path, d / name)
        else:
            (d / name).symlink_to(path)
        (d / "index.html").write_text(
            '<!doctype html><meta charset="utf-8"><style>html,body{margin:0;height:100%;background:#000;overflow:hidden}'
            f'img{{width:100%;height:100%;object-fit:cover}}</style><img src="{name}">')
        prev = _thumb(path, d / "preview.jpg", video=False)
        _write_project(d, _meta(d, {"type": "web", "file": "index.html", "title": path.stem, "source": "Local"}, prev))
    except Exception:
        shutil.rmtree(d, ignore_errors=True)
        raise
    return d


def import_web(path: Path) -> Path:
    """A folder with index.html (or the index.html file itself) is copied in as a web wallpaper."""
    src = path.resolve()
    entry = "index.html"
    if src.is_file():
        entry, src = src.name, src.parent
    if not (src / entry).is_file():
        raise ImportError_(f"No index.html in {src}")
    d = _new_dir(src.name)
    try:
        shutil.copytree(src, d, dirs_exist_ok=True, symlinks=False)
        prev = next((p.name for p in d.glob("preview.*") if p.is_file()), None)
        _write_project(d, _meta(d, {"type": "web", "file": entry, "title": src.name, "source": "Local"}, prev))
    except Exception:
        shutil.rmtree(d, ignore_errors=True)
        raise
    return d


def safe_extract(zf: zipfile.ZipFile, dest: Path, max_bytes: int = MAX_UNPACKED) -> None:
    """Extract regular files only; reject absolute paths, '..', and oversized archives."""
    infos = zf.infolist()
    if len(infos) > MAX_FILES:
        raise ImportError_("Archive has too many files")
    total = 0
    root = dest.resolve()
    plan = []
    for zi in infos:
        name = zi.filename.replace("\\", "/")
        p = PurePosixPath(name)
        if p.is_absolute() or ".." in p.parts or re.match(r"^[A-Za-z]:", name):
            raise ImportError_(f"Unsafe path in archive: {zi.filename}")
        if (zi.external_attr >> 16) & 0o170000 == 0o120000:
            raise ImportError_(f"Symlink in archive: {zi.filename}")
        total += zi.file_size
        if total > max_bytes:
            raise ImportError_(f"Archive unpacks to more than {max_bytes // 1024 ** 2} MiB")
        target = (root / name).resolve()
        if root != target and root not in target.parents:
            raise ImportError_(f"Unsafe path in archive: {zi.filename}")
        plan.append((zi, target, name.endswith("/")))
    for zi, target, is_dir in plan:
        if is_dir:
            target.mkdir(parents=True, exist_ok=True)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        written = 0
        with zf.open(zi) as src, open(target, "wb") as out:  # enforce the cap on real bytes too
            while chunk := src.read(1 << 20):
                written += len(chunk)
                if written > zi.file_size + 1 or written > max_bytes:
                    raise ImportError_("Archive entry larger than declared")
                out.write(chunk)


def map_lively(info: dict) -> dict:
    """LivelyInfo.json -> project.json fields. Raises ImportError_ for unsupported types."""
    t = lively_type(info.get("Type"))
    fname = info.get("FileName")
    title = str(info.get("Title") or "Lively wallpaper")
    if t in _UNSUPPORTED:
        raise ImportError_(f"Lively '{title}' is a {_UNSUPPORTED[t]} wallpaper; WallForge can only run video, GIF, web and picture wallpapers.")
    if t not in ("video", "gif", "web", "webaudio", "picture"):
        raise ImportError_(f"Unknown Lively wallpaper type {info.get('Type')!r}")
    if not fname or not isinstance(fname, str):
        raise ImportError_("LivelyInfo.json has no FileName")
    out = {"type": {"video": "video", "gif": "video", "picture": "web"}.get(t, "web"), "file": fname, "title": title,
           "source": "Lively", "author": str(info.get("Author") or ""), "description": str(info.get("Desc") or ""),
           "license": str(info.get("License") or ""), "source_url": str(info.get("Contact") or ""),
           "tags": [str(x) for x in (info.get("Tags") or [])] if isinstance(info.get("Tags"), list) else [],
           "lively_type": t}
    if info.get("Thumbnail"):
        out["preview"] = str(info["Thumbnail"])
    return out


def import_lively(path: Path) -> Path:
    try:
        zf = zipfile.ZipFile(path)
    except zipfile.BadZipFile:
        raise ImportError_(f"{path.name} is not a valid zip file")
    with zf:
        names = {n.replace("\\", "/") for n in zf.namelist()}
        info_name = next((n for n in sorted(names, key=lambda s: s.count("/")) if n.rsplit("/", 1)[-1].lower() == "livelyinfo.json"), None)
        if not info_name:
            raise ImportError_("Not a Lively package: no LivelyInfo.json")
        try:
            info = json.loads(zf.read(next(zi for zi in zf.infolist() if zi.filename.replace("\\", "/") == info_name)).decode("utf-8-sig"))
        except Exception:
            raise ImportError_("LivelyInfo.json is not valid JSON")
        if not isinstance(info, dict):
            raise ImportError_("LivelyInfo.json is not valid")
        proj = map_lively(info)  # fails early for unsupported types
        prefix = info_name[: -len("LivelyInfo.json")]
        d = _new_dir(proj["title"])
        try:
            safe_extract(zf, d)
            base = d / prefix if prefix else d
            if prefix:  # package nested in a folder: lift it up
                for child in base.iterdir():
                    shutil.move(str(child), d / child.name)
            if not (d / proj["file"]).resolve().is_relative_to(d.resolve()):
                raise ImportError_("FileName escapes the package")
            if not (d / proj["file"]).is_file():
                raise ImportError_(f"Package is missing its wallpaper file {proj['file']!r}")
            if proj.get("preview") and not (d / proj["preview"]).resolve().is_relative_to(d.resolve()):
                proj.pop("preview")
            if proj["lively_type"] == "picture":  # pictures run as a page showing the image
                img = proj["file"]
                (d / "index.html").write_text(
                    '<!doctype html><meta charset="utf-8"><style>html,body{margin:0;height:100%;background:#000;overflow:hidden}'
                    f'img{{width:100%;height:100%;object-fit:cover}}</style><img src="{img}">')
                proj["file"] = "index.html"
            if not proj.get("preview") or not (d / proj["preview"]).is_file():
                proj.pop("preview", None)
                if proj["lively_type"] in ("video", "gif"):
                    p = _thumb(d / proj["file"], d / "preview.jpg")
                    if p:
                        proj["preview"] = p
            _write_project(d, proj)
        except Exception:
            shutil.rmtree(d, ignore_errors=True)
            raise
    return d


def import_path(path: Path, copy: bool = False) -> Path:
    """Import a video, image, HTML folder/file or Lively .zip. Returns the new library folder."""
    path = Path(path).expanduser()
    if not path.exists():
        raise ImportError_(f"{path} does not exist")
    ext = path.suffix.lower()
    if path.is_dir():
        return import_web(path)
    if ext == ".zip":
        return import_lively(path)
    if ext in (".html", ".htm"):
        return import_web(path)
    if ext in VIDEO_EXT:
        return import_video(path, copy)
    if ext in IMAGE_EXT:
        return import_image(path, copy)
    raise ImportError_(f"Unsupported file type {ext or path.name!r}. Use a video, image, HTML folder or Lively .zip.")


def is_local(d: Path) -> bool:
    try:
        return d.resolve().parent == paths.local_dir().resolve() and d.name.startswith("local-")
    except Exception:
        return False


def remove(d: Path) -> None:
    """Delete an imported wallpaper. Refuses anything outside the local library, so Workshop content is safe.
    A video symlink is unlinked; the original file is never touched."""
    if not is_local(d):
        raise ImportError_("Only wallpapers added to WallForge can be removed")
    shutil.rmtree(d)  # rmtree unlinks symlinks without following them


def main(argv: list[str]) -> int:
    copy = "--copy" in argv
    args = [a for a in argv if a != "--copy"]
    if not args:
        print("usage: wallforge --import PATH... [--copy]", file=sys.stderr)
        return 2
    rc = 0
    for a in args:
        try:
            print(import_path(Path(a), copy))
        except ImportError_ as e:
            print(f"{a}: {e}", file=sys.stderr)
            rc = 1
    return rc
