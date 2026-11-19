"""Work around linux-wallpaperengine ignoring user-supplied images (scenetexture properties).

The engine resolves a material's `usertextures: ["customimageleft"]` as `materials/customimageleft.tex`
inside scene.pkg and never reads the property value. We build a patched copy of the wallpaper whose
scene.pkg also holds those .tex files made from the user's images (first frame of GIFs only).
"""
from __future__ import annotations

import hashlib
import os
import shutil
import struct
from pathlib import Path

CACHE = Path.home() / ".cache/wallforge/patched"


def _read_pkg(data: bytes):
    o = 0

    def s():
        nonlocal o
        n = struct.unpack_from("<i", data, o)[0]
        o += 4
        v = data[o:o + n]
        o += n
        return v
    magic = s()
    count = struct.unpack_from("<i", data, o)[0]
    o += 4
    ents = []
    for _ in range(count):
        name = s()
        off, size = struct.unpack_from("<ii", data, o)
        o += 8
        ents.append((name, off, size))
    return magic, [(n, data[o + off:o + off + sz]) for n, off, sz in ents]


def _write_pkg(magic: bytes, files) -> bytes:
    hdr = struct.pack("<i", len(magic)) + magic + struct.pack("<i", len(files))
    off, body = 0, b""
    for name, blob in files:
        hdr += struct.pack("<i", len(name)) + name + struct.pack("<ii", off, len(blob))
        off += len(blob)
        body += blob
    return hdr + body


def _tex(path: str) -> bytes:
    from PIL import Image
    im = Image.open(path).convert("RGBA")
    w, h = im.size
    px = im.tobytes()
    b = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", 0, 0, w, h, w, h, 0)
    b += b"TEXB0003\0" + struct.pack("<Ii", 1, -1) + struct.pack("<I", 1)
    b += struct.pack("<IIIii", w, h, 0, len(px), len(px)) + px
    return b


def patched_dir(run_dir: Path, textures: dict[str, str]) -> Path | None:
    """textures: property name -> image file path. Returns the dir to run, or None when nothing to patch."""
    textures = {k: v for k, v in textures.items() if v and Path(v).is_file()}
    pkg = run_dir / "scene.pkg"
    if not textures or not pkg.is_file():
        return None
    sig = hashlib.sha1(repr((str(run_dir), pkg.stat().st_mtime_ns, sorted((k, v, os.path.getmtime(v)) for k, v in textures.items()))).encode()).hexdigest()[:16]
    out = CACHE / f"{run_dir.name}-{sig}"
    if (out / "scene.pkg").is_file():
        return out
    tmp = CACHE / f".tmp-{os.getpid()}"
    shutil.rmtree(tmp, ignore_errors=True)
    tmp.mkdir(parents=True)
    for f in run_dir.iterdir():
        if f.name != "scene.pkg" and f.is_file():
            shutil.copy2(f, tmp / f.name)
    magic, files = _read_pkg(pkg.read_bytes())
    have = {n for n, _ in files}
    for prop, path in textures.items():
        name = f"materials/{prop}.tex".encode()
        if name in have:
            files = [(n, b) for n, b in files if n != name]
        files.append((name, _tex(path)))
    (tmp / "scene.pkg").write_bytes(_write_pkg(magic, files))
    shutil.rmtree(out, ignore_errors=True)
    tmp.rename(out)
    # drop stale variants of the same wallpaper
    for old in CACHE.glob(f"{run_dir.name}-*"):
        if old != out:
            shutil.rmtree(old, ignore_errors=True)
    return out
