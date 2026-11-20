"""Connected outputs on niri, Hyprland, Sway/wlroots, GNOME (Mutter), KDE (kscreen) or any Wayland/X11 session (GDK fallback)."""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess


def _run(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=5).stdout


def _niri():
    data = json.loads(_run(["niri", "msg", "-j", "outputs"]))
    res = []
    for name, o in data.items():
        lg = o.get("logical") or {}
        mode = next((m for m in o.get("modes", []) if m.get("is_preferred")), {})
        res.append({"name": name, "width": lg.get("width") or mode.get("width", 0),
                    "height": lg.get("height") or mode.get("height", 0)})
    return res


def _hypr():
    res = []
    for o in json.loads(_run(["hyprctl", "-j", "monitors"])):
        sc = o.get("scale") or 1
        res.append({"name": o["name"], "width": int(o["width"] / sc), "height": int(o["height"] / sc)})
    return res


def _sway():
    res = []
    for o in json.loads(_run(["swaymsg", "-t", "get_outputs", "-r"])):
        if o.get("active", True):
            r = o.get("rect") or {}
            res.append({"name": o["name"], "width": r.get("width", 0), "height": r.get("height", 0)})
    return res


def _wlr():
    res, cur = [], None
    for line in _run(["wlr-randr"]).splitlines():
        if line and not line[0].isspace():
            cur = {"name": line.split()[0], "width": 0, "height": 0}
            res.append(cur)
        elif cur and "current" in line:
            m = re.search(r"(\d+)x(\d+)", line)
            if m:
                cur["width"], cur["height"] = int(m.group(1)), int(m.group(2))
    return [r for r in res if r["width"]]


def _mutter():
    """GNOME: connectors and logical sizes from org.gnome.Mutter.DisplayConfig."""
    from gi.repository import Gio
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    r = bus.call_sync("org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig", "org.gnome.Mutter.DisplayConfig",
                      "GetCurrentState", None, None, Gio.DBusCallFlags.NONE, 3000, None).unpack()
    _serial, monitors, logical, _props = r
    modes = {}
    for conn, _vendor_info, mlist, _p in ((m[0][0], m[0], m[1], m[2]) for m in monitors):
        cur = next((m for m in mlist if m[6].get("is-current")), None)
        if cur:
            modes[conn] = (cur[1], cur[2])
    res = []
    for lm in logical:
        x, y, scale, _tr, _prim, mons, _lp = lm
        for mon in mons:
            conn = mon[0]
            w, h = modes.get(conn, (0, 0))
            if w:
                res.append({"name": conn, "width": int(w / scale), "height": int(h / scale)})
    return res


def _gdk():
    import gi
    gi.require_version("Gdk", "4.0")
    from gi.repository import Gdk
    res = []
    mons = Gdk.Display.get_default().get_monitors()
    for i in range(mons.get_n_items()):
        m = mons.get_item(i)
        g = m.get_geometry()
        res.append({"name": m.get_connector() or f"monitor-{i}", "width": g.width, "height": g.height})
    return res


_BACKENDS = (("niri", _niri), ("hyprctl", _hypr), ("swaymsg", _sway), ("wlr-randr", _wlr))


def outputs() -> list[dict]:
    if "GNOME" in os.environ.get("XDG_CURRENT_DESKTOP", "").upper():
        try:
            res = _mutter()
            if res:
                return _tag(res)
        except Exception:
            pass
    for exe, fn in _BACKENDS:
        if shutil.which(exe):
            try:
                res = fn()
                if res:
                    return _tag(res)
            except Exception:
                continue
    try:
        return _tag(_gdk())
    except Exception:
        return []


def _tag(res: list[dict]) -> list[dict]:
    for o in res:
        h = o["height"] or 1
        o["aspect"] = o["width"] / h
        o["ultrawide"] = o["aspect"] >= 2.0
    return res
