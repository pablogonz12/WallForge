"""Persistent settings (~/.config/wallforge/config.json)."""
from __future__ import annotations

import copy
import json
import os
import tempfile
from pathlib import Path

CONFIG_DIR = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "wallforge"
CONFIG_FILE = CONFIG_DIR / "config.json"

DEFAULTS = {
    "version": 5,
    "engine_dir": "",  # empty -> auto-detect
    "renderer": "native",  # native (wallforge-render, used where it can) | legacy (linux-wallpaperengine only)
    "assets_dir": "",  # empty -> auto (own download, then Steam)
    "palette_backend": "auto",  # auto|noctalia|matugen|wallust|pywal|custom|none
    "palette_command": "",  # custom: shell command, {image} is replaced
    "show_anime": False,
    "ultrawide_on_16_9": True,  # allow ultrawide wallpapers on 16:9 screens (cropped to fill)
    "safe_content": True,  # Discover hides Mature/Questionable
    "autostart": False,
    "rotate_minutes": 0,  # change the wallpaper every N minutes, 0 = never
    "rotate_shuffle": True,  # pick at random instead of the next wallpaper in the library
    "rotate_favorites": False,  # rotate only through favourites
    "rotate_per_output": True,  # each screen gets its own pick
    "last_rotate": 0,  # unix time of the last automatic change
    "check_updates": True,
    "auto_update": True,
    "last_update_check": 0,
    "window": {"w": 1280, "h": 800},
    "engine": {
        "scaling": "auto",  # auto (best fit per wallpaper and screen) | fill (crop) | fit | stretch | default
        "clamp": "clamp",  # clamp|border|repeat
        "fps": 30,
        "render_scale": 100,  # render resolution in percent, 25..100
        "reduce_on_battery": True,  # lower fps and resolution while on battery
        "pause_on_battery": False,  # stop wallpapers (and do not restart them) while on battery
        "layer": "background",  # background|bottom (background is what niri's blur/xray and backdrop sample)
        "silent": False,
        "volume": 100,
        "preload_next": True,  # keep the next wallpaper loaded on the layer below, so switching is instant
        "no_audio_processing": False,
        "disable_particles": False,
        "disable_mouse": True,
        "disable_parallax": False,
        "no_fullscreen_pause": False,
        "fullscreen_pause_only_active": True,
    },
    "outputs": {},  # connector -> {"id": item id}
    "values": {},  # item id -> {property: value} user overrides
    "volumes": {},  # item id -> 0..100, the wallpaper's own volume (scaled by the global one)
    "favorites": [],
    "tags_seen": [],
}


def _merge(base, over):
    out = copy.deepcopy(base)
    for k, v in (over or {}).items():
        if isinstance(v, dict) and isinstance(out.get(k), dict) and k not in ("outputs", "values", "volumes"):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


class Config:
    def __init__(self):
        self.data = copy.deepcopy(DEFAULTS)
        self.load()

    def load(self):
        try:
            with open(CONFIG_FILE, encoding="utf-8") as fh:
                raw = json.load(fh)
            self.data = _merge(DEFAULTS, raw)
            self.data["version"] = raw.get("version", 1)
        except FileNotFoundError:
            self.data = _merge(DEFAULTS, {})
        except Exception:
            self.data = _merge(DEFAULTS, {})
        if self.data.get("version", 1) < 2:  # v2: crop ("fill") became the default scaling
            if self.data["engine"].get("scaling") == "default":
                self.data["engine"]["scaling"] = "fill"
            self.data["version"] = 2
            self.save()
        if self.data.get("version", 1) < 3:  # v3: "auto" became the default scaling; move users still on the old default
            if self.data["engine"].get("scaling") == "fill":
                self.data["engine"]["scaling"] = "auto"
            self.data["version"] = 3
            self.save()
        if self.data.get("version", 1) < 4:  # v4: ultrawide wallpapers are allowed on 16:9 screens (cropped)
            self.data["ultrawide_on_16_9"] = True
            self.data["version"] = 4
            self.save()
        if self.data.get("version", 1) < 5:  # v5: render scale and battery saving; the new engine keys come from defaults
            self.data["version"] = 5
            self.save()

    def save(self):
        CONFIG_DIR.mkdir(parents=True, exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=CONFIG_DIR, prefix="config.", suffix=".tmp")
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(self.data, fh, indent=2, sort_keys=True)
        os.replace(tmp, CONFIG_FILE)

    # convenience
    def values_for(self, item_id: str) -> dict:
        return self.data["values"].get(item_id, {})

    def set_value(self, item_id: str, key: str, value):
        self.data["values"].setdefault(item_id, {})[key] = value

    def reset_values(self, item_id: str):
        self.data["values"].pop(item_id, None)
        self.data["volumes"].pop(item_id, None)

    def volume_for(self, item_id: str) -> int:
        """Effective volume 0..100: the global volume scaled by the wallpaper's own; 0 when muted."""
        e = self.data["engine"]
        if e.get("silent"):
            return 0
        try:
            own = max(0, min(100, int(self.data["volumes"].get(item_id, 100))))
            return round(max(0, min(100, int(e.get("volume", 100)))) * own / 100)
        except (TypeError, ValueError):
            return int(e.get("volume", 100))
