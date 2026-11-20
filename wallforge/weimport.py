"""Import settings from Wallpaper Engine's own config.json (the Windows app's settings file).

The file sits in the wallpaper_engine install folder. Layout (one section per Windows user):
  {"?installdirectory": "...",
   "<user>": {"general": {"user": {"fps": 30, "playbackfullscreen": "pause", ...},
                          "playlists": [...],
                          "wallpaperconfig": {"selectedwallpapers": {"<monitor>": {"file": "<path>"}}}},
              "wproperties": {"<path>": {"<monitor>": {"<property>": value}}},
              "wallpaperproperties": [{"file": "<path>", "monitors": {"<monitor>": {...}}}]}}   # older versions
Wallpapers are keyed by Windows paths; only the Workshop id (or project folder name) in the path is used.
"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

from . import library, paths

# Per-wallpaper settings Wallpaper Engine stores next to the wallpaper's own properties.
BUILTIN = {"volume", "rate", "alignment", "alignmentposition", "alignmentx", "alignmenty", "alignmentz", "alignmentfliph",
           "alignmentflipv", "audioprocessing", "parallaxdelay", "wec_e", "wec_brs", "wec_con", "wec_sa", "wec_hue"}
_WORKSHOP = re.compile(r"[/\\]431960[/\\](\d+)(?:[/\\]|$)")
_LOCAL = re.compile(r"[/\\]projects[/\\](?:myprojects|defaultprojects)[/\\]([^/\\]+)(?:[/\\]|$)", re.I)
_MONITOR_N = re.compile(r"monitor(\d+)$", re.I)


def find_configs() -> list[Path]:
    """config.json of every Wallpaper Engine install found in the Steam libraries."""
    return [d / "config.json" for d in paths.we_dirs() if (d / "config.json").is_file()]


def item_id(path: str) -> str | None:
    """Library id for a wallpaper path as Wallpaper Engine writes it (any drive, either slash)."""
    m = _WORKSHOP.search(path) or _LOCAL.search(path)
    return m.group(1) if m else None


def _num(v):
    if isinstance(v, bool):
        return int(v)
    if isinstance(v, (int, float)):
        return v
    try:
        f = float(str(v).strip())
        return int(f) if f.is_integer() else f
    except ValueError:
        return None


def convert(spec: dict, v):
    """A Wallpaper Engine property value as this app stores it, or None when it cannot be used here."""
    if isinstance(v, dict):
        v = v.get("value")
    if v is None or not isinstance(spec, dict):
        return None
    t = str(spec.get("type", "")).lower()
    if t == "bool":
        return v.strip().lower() in ("1", "true") if isinstance(v, str) else bool(v)
    if t == "slider":
        return _num(v)
    if t == "color":
        parts = str(v).replace(",", " ").split()
        try:
            return " ".join(str(float(p)) for p in parts) if len(parts) >= 3 else None
        except ValueError:
            return None
    if t == "combo":
        for o in spec.get("options") or []:
            if isinstance(o, dict) and str(o.get("value")) == str(v):
                return o.get("value")
        return None
    if t == "textinput":
        return str(v)
    return None  # file, scenetexture, directory: Windows paths that do not exist here


@dataclass
class Plan:
    source: Path
    values: dict = field(default_factory=dict)      # item id -> {property: value}
    volumes: dict = field(default_factory=dict)     # item id -> 0..100
    engine: dict = field(default_factory=dict)      # global playback settings to set
    selected: list = field(default_factory=list)    # (Wallpaper Engine monitor key, item id), installed ones only
    missing: set = field(default_factory=set)       # wallpaper ids the config mentions that are not installed here
    skipped: dict = field(default_factory=dict)     # built-in per-wallpaper setting -> times seen, not supported here
    playlists: list = field(default_factory=list)   # (name, number of wallpapers)

    @property
    def empty(self) -> bool:
        return not (self.values or self.volumes or self.engine or self.selected)

    def assignments(self, outputs: list[str]) -> dict:
        """output -> item id, only where Wallpaper Engine's monitors map onto ours without guessing."""
        ids = [i for _m, i in self.selected]
        if not ids or not outputs:
            return {}
        if len(set(ids)) == 1:
            return {o: ids[0] for o in outputs}
        nums = [(_MONITOR_N.search(m), i) for m, i in self.selected]
        if all(m for m, _i in nums) and len(nums) <= len(outputs):
            ordered = sorted((int(m.group(1)), i) for m, i in nums)
            return {outputs[n]: i for n, (_k, i) in enumerate(ordered)}
        return {}

    def summary(self, titles: dict | None = None) -> list[str]:
        titles = titles or {}
        out = []
        if self.values:
            n = sum(len(v) for v in self.values.values())
            out.append(f"{n} customised setting{'s' * (n != 1)} for {len(self.values)} wallpaper{'s' * (len(self.values) != 1)}")
        if self.volumes:
            out.append(f"Volume for {len(self.volumes)} wallpaper{'s' * (len(self.volumes) != 1)}")
        if "fps" in self.engine:
            out.append(f"Frame rate limit: {self.engine['fps']}")
        if "no_fullscreen_pause" in self.engine:
            out.append("Under fullscreen apps: " + ("keep playing" if self.engine["no_fullscreen_pause"] else "pause"))
        if "silent" in self.engine:
            out.append("Sound: " + ("muted" if self.engine["silent"] else "on"))
        if self.selected:
            names = list(dict.fromkeys(titles.get(i, i) for _m, i in self.selected))
            out.append("Active in Wallpaper Engine: " + ", ".join(names[:3]) + (f" and {len(names) - 3} more" if len(names) > 3 else ""))
        return out

    def notes(self) -> list[str]:
        out = []
        if self.missing:
            out.append(f"{len(self.missing)} wallpaper{'s' * (len(self.missing) != 1)} in the file are not installed here and were left out")
        if self.playlists:
            out.append(f"{len(self.playlists)} playlist{'s' * (len(self.playlists) != 1)} not imported (playlists are not supported yet)")
        if self.skipped:
            kinds = sorted({"colour adjustments" if k.startswith("wec_") else "alignment" if k.startswith("alignment")
                            else {"rate": "playback speed", "audioprocessing": "audio response", "parallaxdelay": "parallax delay"}.get(k, k)
                            for k in self.skipped})
            out.append("Per-wallpaper " + ", ".join(kinds) + " not imported (not supported yet)")
        return out


def _users(root: dict) -> list[dict]:
    return [v for k, v in root.items() if not k.startswith("?") and isinstance(v, dict)
            and any(x in v for x in ("general", "wproperties", "wallpaperproperties"))]


def _per_wallpaper(user: dict):
    """Yield (path, {monitor: {property: value}}) from both the current and the older layout."""
    for e in user.get("wallpaperproperties") or []:
        if isinstance(e, dict) and isinstance(e.get("file"), str) and isinstance(e.get("monitors"), dict):
            yield e["file"], e["monitors"]
    wp = user.get("wproperties")
    if isinstance(wp, dict):
        for path, mons in wp.items():
            if isinstance(mons, dict):
                yield path, mons


def read(path: Path) -> dict:
    try:
        with open(path, encoding="utf-8-sig") as fh:
            root = json.load(fh)
    except json.JSONDecodeError:
        raise ValueError("That file is not valid JSON, so it is not a Wallpaper Engine config.json") from None
    if not isinstance(root, dict) or not _users(root):
        raise ValueError("This is not a Wallpaper Engine config.json")
    return root


def plan(path: Path, items: list[library.Item] | None = None) -> Plan:
    """Work out what a config.json would change. Nothing is written until apply()."""
    root = read(path)
    by_id = {i.id: i for i in (library.scan() if items is None else items)}
    p = Plan(source=Path(path))
    for user in _users(root):
        general = user.get("general") if isinstance(user.get("general"), dict) else {}
        sel = ((general.get("wallpaperconfig") or {}).get("selectedwallpapers") or {})
        active = {}  # item id -> monitor keys it is selected on
        for mon, entry in sel.items() if isinstance(sel, dict) else ():
            i = item_id(entry.get("file", "")) if isinstance(entry, dict) and isinstance(entry.get("file"), str) else None
            if not i:
                continue
            if i in by_id:
                active.setdefault(i, []).append(mon)
                if (mon, i) not in p.selected:
                    p.selected.append((mon, i))
            else:
                p.missing.add(i)
        for wpath, mons in _per_wallpaper(user):
            i = item_id(wpath)
            if not i:
                continue
            if i not in by_id:
                p.missing.add(i)
                continue
            it = by_id[i]
            specs, defaults = it.properties(), it.defaults()
            # monitors where the wallpaper is active win over stale entries for monitors long gone
            order = sorted(mons, key=lambda m: m in active.get(i, []))
            for mon in order:
                vals = mons[mon]
                if not isinstance(vals, dict):
                    continue
                for k, v in vals.items():
                    if k in specs:
                        c = convert(specs[k], v)
                        if c is None:
                            continue
                        if c == defaults.get(k) and k not in p.values.get(i, {}):
                            continue
                        p.values.setdefault(i, {})[k] = c
                    elif k == "volume":
                        n = _num(v)
                        if n is not None:
                            p.volumes[i] = max(0, min(100, int(n)))
                    elif k in BUILTIN:
                        p.skipped[k] = p.skipped.get(k, 0) + 1
        u = general.get("user") if isinstance(general.get("user"), dict) else {}
        fps = _num(u.get("fps"))
        if fps:
            p.engine["fps"] = max(5, min(240, int(fps)))
        if u.get("playbackfullscreen") in ("run", "pause", "stop"):
            p.engine["no_fullscreen_pause"] = u["playbackfullscreen"] == "run"
        for pl in general.get("playlists") or []:
            if isinstance(pl, dict) and pl.get("items"):
                p.playlists.append((str(pl.get("name") or "unnamed"), len(pl["items"])))
    for i in [k for k, v in p.volumes.items() if v == 100]:
        del p.volumes[i]  # 100 is the default, nothing to remember
    return p


def apply(cfg, p: Plan):
    """Merge a plan into the config and save it. Existing values for other wallpapers are kept."""
    d = cfg.data
    for i, vals in p.values.items():
        d["values"].setdefault(i, {}).update(vals)
    d.setdefault("volumes", {}).update(p.volumes)
    d["engine"].update(p.engine)
    cfg.save()


def main(argv: list[str]) -> int:
    """wallforge --import-we [config.json] [--dry-run]"""
    from . import config
    dry = "--dry-run" in argv
    args = [a for a in argv if not a.startswith("--")]
    found = [Path(args[0])] if args else find_configs()
    if not found:
        print("No Wallpaper Engine config.json found. Pass its path: wallforge --import-we /path/to/config.json")
        return 1
    try:
        p = plan(found[0])
    except (OSError, ValueError) as e:
        print(f"{found[0]}: {e}")
        return 1
    titles = {i.id: i.title for i in library.scan()}
    print(f"From {p.source}:")
    for line in p.summary(titles) or ["Nothing to import"]:
        print("  " + line)
    for line in p.notes():
        print("  (" + line + ")")
    if not dry and not p.empty:
        apply(config.Config(), p)
        print("Imported.")
    return 0
