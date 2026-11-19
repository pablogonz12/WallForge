"""Property editor: builds libadwaita rows from a Wallpaper Engine project.json."""
from __future__ import annotations

import re
from typing import Callable

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
from gi.repository import Adw, Gdk, Gio, GLib, Gtk  # noqa: E402


def parse_color(v) -> Gdk.RGBA:
    c = Gdk.RGBA()
    try:
        r, g, b = [float(x) for x in str(v).replace(",", " ").split()[:3]]
        c.red, c.green, c.blue, c.alpha = r, g, b, 1.0
    except Exception:
        c.red = c.green = c.blue = 0.0
        c.alpha = 1.0
    return c


def color_str(c: Gdk.RGBA) -> str:
    return f"{c.red:.6f} {c.green:.6f} {c.blue:.6f}"


def eval_condition(cond: str | None, values: dict) -> bool:
    """WE conditions look like `name.value == 1 && other.value`. Unknown syntax -> visible."""
    if not cond:
        return True
    try:
        def sub(m):
            v = values.get(m.group(1))
            if isinstance(v, bool):
                return "True" if v else "False"
            if isinstance(v, (int, float)):
                return repr(v)
            if v is None:
                return "None"
            return repr(str(v))
        expr = re.sub(r"([A-Za-z_]\w*)\.value", sub, cond)
        expr = expr.replace("&&", " and ").replace("||", " or ")
        expr = re.sub(r"(?<![=!<>])!(?!=)", " not ", expr)
        expr = re.sub(r"\btrue\b", "True", expr)
        expr = re.sub(r"\bfalse\b", "False", expr)
        if re.search(r"[^\w\s.=!<>'\"()\-+*/]", expr):
            return True
        return bool(eval(expr, {"__builtins__": {}}, {}))  # noqa: S307 - sanitised above
    except Exception:
        return True


class PropertyEditor(Gtk.Box):
    """`values` is the live dict (defaults then overrides). `on_change(key, value)` fires per edit."""

    def __init__(self, props: dict, values: dict, on_change: Callable, parent_window: Gtk.Window):
        super().__init__(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        self.props, self.values, self.on_change, self.win = props, values, on_change, parent_window
        self.rows: dict[str, Gtk.Widget] = {}
        group = None
        ordered = sorted(
            ((k, s) for k, s in props.items() if isinstance(s, dict)),
            key=lambda kv: kv[1].get("order", 0),
        )
        for key, spec in ordered:
            t = (spec.get("type") or "").lower()
            label = re.sub(r"<[^>]+>", "", str(spec.get("text") or key)).strip() or key
            if t == "group":
                group = Adw.PreferencesGroup(title=GLib.markup_escape_text(label))
                self.append(group)
                self.rows[key] = group
                continue
            if t == "text":
                if not label or label == key:
                    continue
                if group is None:
                    group = Adw.PreferencesGroup()
                    self.append(group)
                row = Adw.ActionRow(title=GLib.markup_escape_text(label))
                row.add_css_class("dim-label")
                group.add(row)
                self.rows[key] = row
                continue
            row = self._make_row(key, spec, t, GLib.markup_escape_text(label))
            if row is None:
                continue
            if group is None:
                group = Adw.PreferencesGroup()
                self.append(group)
            group.add(row)
            self.rows[key] = row
        self.refresh_visibility()

    def refresh_visibility(self):
        for key, w in self.rows.items():
            spec = self.props.get(key, {})
            w.set_visible(eval_condition(spec.get("condition"), self.values))

    def _set(self, key, value):
        self.values[key] = value
        self.on_change(key, value)
        self.refresh_visibility()

    def _make_row(self, key, spec, t, label):
        cur = self.values.get(key, spec.get("value"))
        if t == "bool":
            row = Adw.SwitchRow(title=label)
            row.set_active(cur in (True, 1, "1", "true", "True"))
            row.connect("notify::active", lambda r, _p: self._set(key, r.get_active()))
            return row
        if t == "slider":
            lo, hi = float(spec.get("min", 0)), float(spec.get("max", 100))
            step = float(spec.get("step", 0) or (hi - lo) / 100 or 1)
            row = Adw.ActionRow(title=label)
            scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, lo, hi, step)
            scale.set_value(float(cur if cur not in (None, "") else lo))
            scale.set_draw_value(True)
            scale.set_value_pos(Gtk.PositionType.RIGHT)
            scale.set_digits(0 if step >= 1 else 2)
            scale.set_hexpand(True)
            scale.set_size_request(180, -1)
            scale.set_valign(Gtk.Align.CENTER)
            as_int = step >= 1 and float(step).is_integer()
            scale.connect("value-changed", lambda s: self._set(key, int(s.get_value()) if as_int else round(s.get_value(), 4)))
            row.add_suffix(scale)
            return row
        if t == "color":
            row = Adw.ActionRow(title=label)
            btn = Gtk.ColorDialogButton(dialog=Gtk.ColorDialog(with_alpha=False))
            btn.set_rgba(parse_color(cur))
            btn.set_valign(Gtk.Align.CENTER)
            btn.connect("notify::rgba", lambda b, _p: self._set(key, color_str(b.get_rgba())))
            row.add_suffix(btn)
            return row
        if t == "combo":
            opts = spec.get("options") or []
            labels = [str(o.get("label", o.get("value"))) for o in opts]
            row = Adw.ComboRow(title=label, model=Gtk.StringList.new(labels))
            for i, o in enumerate(opts):
                if str(o.get("value")) == str(cur):
                    row.set_selected(i)
            row.connect("notify::selected", lambda r, _p: self._set(key, opts[r.get_selected()].get("value")))
            return row
        if t == "textinput":
            row = Adw.EntryRow(title=label)
            row.set_text(str(cur or ""))
            row.connect("changed", lambda r: self._set(key, r.get_text()))
            return row
        if t in ("scenetexture", "file"):
            return self._file_row(key, label, cur, t)
        return Adw.ActionRow(title=label, subtitle=f"{t or 'unknown'} (not editable)")

    def _file_row(self, key, label, cur, t):
        row = Adw.ActionRow(title=label, subtitle=self._short(cur))
        row.set_subtitle_lines(1)
        choose = Gtk.Button(icon_name="document-open-symbolic", valign=Gtk.Align.CENTER, tooltip_text="Choose file")
        clear = Gtk.Button(icon_name="edit-clear-symbolic", valign=Gtk.Align.CENTER, tooltip_text="Clear")
        clear.add_css_class("flat")

        def done(dlg, res):
            try:
                f = dlg.open_finish(res)
            except GLib.Error:
                return
            path = f.get_path()
            row.set_subtitle(self._short(path))
            self._set(key, path)

        def pick(_b):
            dlg = Gtk.FileDialog(title=label)
            flt = Gtk.FileFilter()
            flt.set_name("Images" if t == "scenetexture" else "Files")
            if t == "scenetexture":
                for pat in ("*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.bmp", "*.tga"):
                    flt.add_pattern(pat)
            else:
                flt.add_pattern("*")
            store = Gio.ListStore.new(Gtk.FileFilter)
            store.append(flt)
            dlg.set_filters(store)
            dlg.open(self.win, None, done)

        def do_clear(_b):
            row.set_subtitle("none")
            self._set(key, "")

        choose.connect("clicked", pick)
        clear.connect("clicked", do_clear)
        row.add_suffix(clear)
        row.add_suffix(choose)
        return row

    @staticmethod
    def _short(v):
        return str(v).replace(str(GLib.get_home_dir()), "~") if v else "none"
