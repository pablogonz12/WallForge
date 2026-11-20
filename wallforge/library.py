"""Scan Wallpaper Engine Workshop items and resolve presets."""
from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass, field
from pathlib import Path

from . import paths

TYPES = ("scene", "video", "web", "application")


@dataclass
class Item:
    id: str
    dir: Path
    title: str
    type: str  # scene | video | web | application | preset | unknown
    preview: Path | None
    tags: list[str] = field(default_factory=list)
    description: str = ""
    dependency: str | None = None
    preset: dict = field(default_factory=dict)
    raw: dict = field(default_factory=dict)
    broken: str | None = None  # reason it cannot run
    aspect: float | None = None  # wallpaper's own aspect ratio when known (scenes)

    # ---- resolution --------------------------------------------------------
    @property
    def is_preset(self) -> bool:
        return not self.raw.get("type") and bool(self.dependency)

    @property
    def run_dir(self) -> Path:
        """Directory the engine must be started on (presets run their base)."""
        if self.is_preset:
            return self.dir.parent / str(self.dependency)
        return self.dir

    @property
    def kind(self) -> str:
        """Effective wallpaper kind for UI filtering."""
        if self.is_preset:
            base = _load_json(self.run_dir / "project.json")
            return (base.get("type") or "unknown").lower() if base else "unknown"
        return self.type

    @property
    def workshop_id(self) -> str | None:
        """Steam Workshop id, or None for a local project that was never published."""
        if self.id.isdigit() and self.dir.parent.name == paths.APPID:
            return self.id
        wid = str(self.raw.get("workshopid") or "")
        return wid if wid.isdigit() else None

    @property
    def ultrawide(self) -> bool:
        if any(t.lower().startswith(("ultrawide", "dual", "triple")) for t in self.tags):
            return True
        return bool(self.aspect and self.aspect >= 2.0)

    @property
    def anime(self) -> bool:
        return any(t.lower() in ("anime", "girls") for t in self.tags)

    def base_project(self) -> dict:
        return _load_json(self.run_dir / "project.json") or self.raw

    def properties(self) -> dict:
        """User-editable property definitions of the wallpaper actually run."""
        return (self.base_project().get("general", {}) or {}).get("properties", {}) or {}

    def defaults(self) -> dict:
        """Property values as they would be without user changes (base ← preset)."""
        out = {}
        for key, spec in self.properties().items():
            if isinstance(spec, dict) and "value" in spec and spec.get("type") not in ("text", "group", "Text"):
                out[key] = spec["value"]
        for key, val in (self.preset or {}).items():
            if key in out and val is not None:
                out[key] = self._abs_file(key, val)
        return out

    def base_defaults(self) -> dict:
        out = {}
        for key, spec in self.properties().items():
            if isinstance(spec, dict) and "value" in spec and spec.get("type") not in ("text", "group", "Text"):
                out[key] = spec["value"]
        return out

    def _abs_file(self, key: str, val):
        spec = self.properties().get(key, {})
        if spec.get("type") in ("scenetexture", "file") and isinstance(val, str) and val:
            p = Path(val)
            if not p.is_absolute():
                p = self.dir / val
            return str(p) if p.exists() else ""
        return val


def _load_json(path: Path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return json.load(fh)
    except Exception:
        return None


def _scan_dir(root: Path) -> list[Item]:
    items: list[Item] = []
    if not root.is_dir():
        return items
    for d in sorted(root.iterdir()):
        proj = d / "project.json"
        if not proj.is_file():
            continue
        raw = _load_json(proj)
        if not isinstance(raw, dict):
            continue
        ptype = (raw.get("type") or "").lower()
        dep = raw.get("dependency")
        if not ptype and dep:
            ptype = "preset"
        prev = raw.get("preview") or "preview.jpg"
        preview = d / prev
        if not preview.is_file():
            cands = [p for p in d.glob("preview.*") if p.is_file()]
            preview = cands[0] if cands else None
        item = Item(
            id=d.name,
            dir=d,
            title=str(raw.get("title") or d.name),
            type=ptype or "unknown",
            preview=preview,
            tags=[str(t) for t in (raw.get("tags") or [])],
            description=re.sub(r"<[^>]+>", "", str(raw.get("description") or "")),
            dependency=str(dep) if dep else None,
            preset=raw.get("preset") or {},
            raw=raw,
        )
        if item.is_preset and not item.run_dir.is_dir():
            item.broken = f"base wallpaper {dep} is not downloaded"
        items.append(item)
    return items


def _scene_json(run_dir: Path):
    """Read scene.json from scene.pkg (header + one entry; the pkg can be hundreds of MB) or from disk."""
    loose = run_dir / "scene.json"
    if loose.is_file():
        return _load_json(loose)
    pkg = run_dir / "scene.pkg"
    if not pkg.is_file():
        return None
    try:
        import struct
        with open(pkg, "rb") as f:
            def s():
                n = struct.unpack("<i", f.read(4))[0]
                if not 0 <= n < 4096:
                    raise ValueError
                return f.read(n)
            s()
            count = struct.unpack("<i", f.read(4))[0]
            ents = []
            for _ in range(count):
                name = s()
                off, size = struct.unpack("<ii", f.read(8))
                ents.append((name, off, size))
            base = f.tell()
            for name, off, size in ents:
                if name == b"scene.json":
                    f.seek(base + off)
                    return json.loads(f.read(size))
    except Exception:
        return None
    return None


def _check_scene(item: Item):
    """Flag scenes the engine cannot load and record the scene's own aspect ratio."""
    if item.broken or item.kind != "scene":
        return
    sc = _scene_json(item.run_dir)
    if not isinstance(sc, dict):
        return
    proj = (sc.get("general") or {}).get("orthogonalprojection")
    if not isinstance(proj, dict) or not sc.get("camera"):
        item.broken = "3D scene: not supported by the Linux engine"
        return
    try:
        if proj.get("auto"):
            item.aspect = None
        elif proj.get("width") and proj.get("height"):
            item.aspect = float(proj["width"]) / float(proj["height"])
    except Exception:
        pass


def scan() -> list[Item]:
    items: list[Item] = []
    seen = set()
    roots = paths.workshop_dirs() + [d / "projects/myprojects" for d in paths.we_dirs()]
    for root in roots:
        for it in _scan_dir(root):
            if it.id not in seen:
                seen.add(it.id)
                items.append(it)
    for it in items:
        _check_scene(it)
    items.sort(key=lambda i: i.title.lower())
    return items


def find(item_id: str) -> Item | None:
    for it in scan():
        if it.id == item_id:
            return it
    return None
