"""WallForge: GTK4 / libadwaita front end for linux-wallpaperengine."""
from __future__ import annotations

import hashlib
import os
import random
import shutil
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import gi

gi.require_version("Gtk", "4.0")
gi.require_version("Adw", "1")
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import Adw, Gdk, GdkPixbuf, Gio, GLib, Gtk  # noqa: E402

from . import (  # noqa: E402
    __version__,  # noqa: E402
    assets,
    config,
    engine,
    library,
    monitors,
    paths,
    rotation,
    theme,
    updater,
    weimport,
    workshop,
)
from .props import PropertyEditor  # noqa: E402

APP_ID = "dev.wallforge.Wallforge"
THUMBS = Path.home() / ".cache/wallforge/thumbs"
POOL = ThreadPoolExecutor(4)

CSS = """
.wf-card { padding: 0; }
.wf-card picture { border-radius: 12px 12px 0 0; }
.wf-title { font-weight: 600; }
.wf-badge { font-size: 0.75em; padding: 1px 8px; border-radius: 99px; background: alpha(currentColor, .12); }
.wf-active { outline: 2px solid @accent_bg_color; outline-offset: -2px; border-radius: 12px; }
"""


def load_thumb(src: str, cb, size=(480, 270)):
    """Load a file path or http URL into a Gdk.Texture off the main thread."""
    def work():
        try:
            path = src
            if src.startswith("http"):
                THUMBS.mkdir(parents=True, exist_ok=True)
                path = str(THUMBS / hashlib.sha1(src.encode()).hexdigest())
                if not Path(path).exists():
                    req = urllib.request.Request(src, headers={"User-Agent": workshop.UA})
                    with urllib.request.urlopen(req, timeout=20) as r:
                        Path(path).write_bytes(r.read())
            pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(path, size[0], size[1], True)
            GLib.idle_add(lambda: cb(Gdk.Texture.new_for_pixbuf(pb)) and False)
        except Exception:
            pass
    POOL.submit(work)


def make_card(title: str, preview: str | None, badge: str = "", extra: str = "") -> Gtk.Box:
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    box.add_css_class("card")
    box.add_css_class("wf-card")
    pic = Gtk.Picture(content_fit=Gtk.ContentFit.COVER, can_shrink=True)
    pic.set_size_request(220, 124)
    pic.set_overflow(Gtk.Overflow.HIDDEN)
    box.append(pic)
    if preview:
        load_thumb(preview, lambda t: pic.set_paintable(t))
    inner = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, margin_top=8, margin_bottom=8, margin_start=10, margin_end=10)
    lab = Gtk.Label(label=title, xalign=0, ellipsize=3, max_width_chars=26)
    lab.add_css_class("wf-title")
    inner.append(lab)
    row = Gtk.Box(spacing=6)
    if badge:
        b = Gtk.Label(label=badge)
        b.add_css_class("wf-badge")
        row.append(b)
    if extra:
        e = Gtk.Label(label=extra, xalign=0)
        e.add_css_class("dim-label")
        e.add_css_class("caption")
        row.append(e)
    inner.append(row)
    box.append(inner)
    return box


class Window(Adw.ApplicationWindow):
    def __init__(self, app, cfg: config.Config):
        win = cfg.data.get("window") or {}
        super().__init__(application=app, title="WallForge", default_width=win.get("w", 1280), default_height=win.get("h", 800))
        self.cfg = cfg
        self.items: list[library.Item] = []
        self.sel: library.Item | None = None
        self.edit_values: dict = {}
        self.dirty = False
        self.web_page = 1
        self.web_items: list[dict] = []
        self.known_ids: set[str] = set()
        self._applied: dict[str, str] = {}  # output -> item id we last started (watchdog memory)
        self._restarts: dict[str, list[float]] = {}  # output -> restart timestamps
        self._started: dict[str, float] = {}  # output -> when we last tried to start it
        self._crashed: set[str] = set()  # outputs given up on after repeated crashes
        self._stopped: set[str] = set()  # outputs the user stopped by hand (rotation leaves them alone)
        self._battery_paused = False
        self._last_rotate = float(self.cfg.data.get("last_rotate") or 0)

        self.toasts = Adw.ToastOverlay()
        self.set_content(self.toasts)
        tv = Adw.ToolbarView()
        self.toasts.set_child(tv)

        self.stack = Adw.ViewStack()
        header = Adw.HeaderBar()
        switcher = Adw.ViewSwitcher(stack=self.stack, policy=Adw.ViewSwitcherPolicy.WIDE)
        header.set_title_widget(switcher)
        menu_btn = Gtk.Button(icon_name="emblem-system-symbolic", tooltip_text="Settings")
        menu_btn.connect("clicked", lambda _b: self.show_settings())
        header.pack_end(menu_btn)
        refresh = Gtk.Button(icon_name="view-refresh-symbolic", tooltip_text="Rescan library")
        refresh.connect("clicked", lambda _b: self.reload_library(toast=True))
        header.pack_start(refresh)
        rnd = Gtk.Button(icon_name="media-playlist-shuffle-symbolic", tooltip_text="Random wallpaper (Ctrl+R)")
        rnd.connect("clicked", lambda _b: self.random_wallpaper())
        header.pack_start(rnd)
        tv.add_top_bar(header)
        self.banner = Adw.Banner(button_label="Set up", revealed=False)
        self.banner.connect("button-clicked", lambda _b: self.show_settings())
        tv.add_top_bar(self.banner)

        self.stack.add_titled_with_icon(self._build_library(), "library", "Library", "folder-pictures-symbolic")
        self.stack.add_titled_with_icon(self._build_discover(), "discover", "Discover", "folder-download-symbolic")
        tv.set_content(self.stack)
        tv.add_bottom_bar(self._build_playing_bar())

        self._shortcuts()
        self.connect("close-request", self._on_close)
        self.reload_library()
        self._check_setup()
        self.refresh_playing()
        if self.cfg.data["check_updates"]:
            if time.time() - self.cfg.data["last_update_check"] > 6 * 3600:
                GLib.timeout_add_seconds(5, lambda: (self.check_update(quiet=True), False)[1])
            GLib.timeout_add_seconds(6 * 3600, self._periodic_update)
        self._building = not paths.find_render() and bool(shutil.which("cargo"))

        def build():
            updater.ensure_render()
            self._building = False
            GLib.idle_add(self._check_setup)

        threading.Thread(target=build, daemon=True).start()
        GLib.timeout_add_seconds(4, self._watch)
        GLib.timeout_add_seconds(3, self._poll_playing)

    # ------------------------------------------------------------------ library
    def _build_library(self):
        self.split = Adw.OverlaySplitView(sidebar_position=Gtk.PackType.END, show_sidebar=False, max_sidebar_width=440, min_sidebar_width=360)
        main = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        bar = Gtk.Box(spacing=8, margin_top=8, margin_bottom=8, margin_start=12, margin_end=12)
        self.search = Gtk.SearchEntry(placeholder_text="Search installed wallpapers", hexpand=True)
        self.search.connect("search-changed", lambda *_: self.filter.changed(Gtk.FilterChange.DIFFERENT))
        self.type_dd = Gtk.DropDown.new_from_strings(["All", "Scene", "Video", "Web", "Favorites"])
        self.type_dd.connect("notify::selected", lambda *_: self.filter.changed(Gtk.FilterChange.DIFFERENT))
        self.sort_dd = Gtk.DropDown.new_from_strings(["A to Z", "Newest first", "Favorites first"])
        self.sort_dd.connect("notify::selected", lambda *_: self.reload_library())
        bar.append(self.search)
        bar.append(self.type_dd)
        bar.append(self.sort_dd)
        main.append(bar)
        self.hidden_lbl = Gtk.Label(xalign=0, margin_start=14, margin_bottom=4, visible=False)
        self.hidden_lbl.add_css_class("dim-label")
        self.hidden_lbl.add_css_class("caption")
        main.append(self.hidden_lbl)
        self.flow = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.SINGLE, homogeneous=True, min_children_per_line=2,
                                max_children_per_line=8, column_spacing=12, row_spacing=12, margin_start=12, margin_end=12,
                                margin_bottom=12, valign=Gtk.Align.START)
        self.flow.set_filter_func(self._flow_filter)
        self.filter = self.flow  # filter_changed via invalidate
        self.filter = _FlowFilter(self.flow)
        self.flow.connect("child-activated", self._on_pick)
        self.empty = Adw.StatusPage(icon_name="folder-pictures-symbolic", title="No wallpapers yet",
                                    description="Open the Discover tab to get wallpapers from the Steam Workshop, "
                                                "or subscribe to them in Steam.", vexpand=True, visible=False)
        sc = Gtk.ScrolledWindow(vexpand=True, child=self.flow)
        self.lib_stack = Gtk.Stack()
        self.lib_stack.add_named(sc, "grid")
        self.lib_stack.add_named(self.empty, "empty")
        main.append(self.lib_stack)
        self.split.set_content(main)

        self.detail_scroll = Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER)
        self.split.set_sidebar(self.detail_scroll)
        return self.split

    def _flow_filter(self, child):
        it: library.Item = child.item
        if self._hidden_reason(it):
            return False
        q = self.search.get_text().strip().lower()
        if q and q not in it.title.lower() and not any(q in t.lower() for t in it.tags):
            return False
        mode = self.type_dd.get_selected_item().get_string()
        if mode == "Favorites":
            return it.id in self.cfg.data["favorites"]
        if mode != "All" and it.kind != mode.lower():
            return False
        return True

    def _has_ultrawide_monitor(self):
        if not hasattr(self, "_mons") or not self._mons:
            self._mons = monitors.outputs()
        return any(o["ultrawide"] for o in self._mons)

    def _hidden_reason(self, it: library.Item):
        if it.anime and not self.cfg.data["show_anime"]:
            return "anime"
        if it.ultrawide and not self.cfg.data["ultrawide_on_16_9"] and not self._has_ultrawide_monitor():
            return "ultrawide"
        return None

    def _update_hidden(self):
        n = {}
        for it in self.items:
            r = self._hidden_reason(it)
            if r:
                n[r] = n.get(r, 0) + 1
        if n:
            self.hidden_lbl.set_text("Hidden: " + ", ".join(f"{v} {k}" for k, v in n.items()) + " (change in Settings)")
        self.hidden_lbl.set_visible(bool(n))
        visible = any(not self._hidden_reason(i) for i in self.items)
        self.lib_stack.set_visible_child_name("grid" if visible else "empty")

    def reload_library(self, toast=False):
        self._mons = monitors.outputs()
        self.items = library.scan()
        mode = self.sort_dd.get_selected()
        if mode == 1:
            self.items.sort(key=lambda i: -(i.dir / "project.json").stat().st_mtime)
        elif mode == 2:
            fav = set(self.cfg.data["favorites"])
            self.items.sort(key=lambda i: (i.id not in fav, i.title.lower()))
        self.known_ids = {i.id for i in self.items}
        child = self.flow.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.flow.remove(child)
            child = nxt
        active = {self._active_id(o["name"]) for o in monitors.outputs()}
        for it in self.items:
            badge = it.kind if not it.is_preset else f"{it.kind} preset"
            if it.broken:
                badge = "unsupported" if it.broken.startswith("3D") else "missing base"
            elif it.ultrawide:
                badge += " 21:9"
            card = make_card(it.title, str(it.preview) if it.preview else None, badge)
            if it.id in active:
                card.add_css_class("wf-active")
            fc = Gtk.FlowBoxChild(child=card)
            fc.item = it
            self.flow.append(fc)
        self._update_hidden()
        self._check_setup()
        if toast:
            self.toasts.add_toast(Adw.Toast(title=f"{len(self.items)} wallpapers"))

    def _active_id(self, output):
        return self.cfg.data["outputs"].get(output, {}).get("id") if engine.running(output) else None

    def _on_pick(self, _flow, child):
        self.select(child.item)

    # ------------------------------------------------------------------ detail
    def select(self, it: library.Item):
        self.sel = it
        self.dirty = False
        defaults = it.defaults()
        defaults.update(self.cfg.values_for(it.id))
        self.edit_values = defaults

        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14, margin_top=14, margin_bottom=14, margin_start=14, margin_end=14)
        pic = Gtk.Picture(content_fit=Gtk.ContentFit.COVER, can_shrink=True, height_request=200)
        pic.set_overflow(Gtk.Overflow.HIDDEN)
        pic.add_css_class("card")
        if it.preview:
            load_thumb(str(it.preview), lambda t: pic.set_paintable(t), (800, 450))
        box.append(pic)
        title = Gtk.Label(label=it.title, xalign=0, wrap=True)
        title.add_css_class("title-2")
        box.append(title)
        meta = f"{it.kind}" + (" preset" if it.is_preset else "") + f"  |  id {it.id}"
        ml = Gtk.Label(label=meta, xalign=0)
        ml.add_css_class("dim-label")
        box.append(ml)
        if it.broken:
            warn = Gtk.Label(label=it.broken, xalign=0, wrap=True)
            warn.add_css_class("error")
            box.append(warn)

        outs = self._mons or monitors.outputs()
        if it.ultrawide:
            note = Gtk.Label(label="Made for ultrawide screens. On 16:9 monitors it is cropped to fill (turn that off in Settings to skip it).",
                             xalign=0, wrap=True)
            note.add_css_class("dim-label")
            box.append(note)
        self.out_dd = Gtk.DropDown.new_from_strings([o["name"] for o in outs])
        grp = Adw.PreferencesGroup()
        row = Adw.ActionRow(title="Monitor")
        row.add_suffix(self.out_dd)
        grp.add(row)
        if it.kind in ("scene", "video", "web"):
            vol = Adw.SpinRow.new_with_range(0, 100, 5)
            vol.set_title("Volume")
            vol.set_subtitle("For this wallpaper only")
            vol.set_value(self.cfg.data["volumes"].get(it.id, 100))
            vol.connect("notify::value", lambda r, _p: self._on_volume(int(r.get_value())))
            grp.add(vol)
        box.append(grp)
        btns = Gtk.Box(spacing=8, homogeneous=True)
        ap = Gtk.Button(label="Apply")
        ap.add_css_class("suggested-action")
        ap.add_css_class("pill")
        ap.connect("clicked", lambda _b: self.apply([self.out_dd.get_selected_item().get_string()]))
        al = Gtk.Button(label="Apply to all")
        al.add_css_class("pill")
        al.connect("clicked", lambda _b: self.apply([o["name"] for o in outs]))
        btns.append(ap)
        btns.append(al)
        box.append(btns)
        tools = Gtk.Box(spacing=8)
        fav = Gtk.ToggleButton(icon_name="starred-symbolic", tooltip_text="Favorite", active=it.id in self.cfg.data["favorites"])
        fav.connect("toggled", self._toggle_fav)
        reset = Gtk.Button(label="Reset settings")
        reset.connect("clicked", lambda _b: self._reset())
        close = Gtk.Button(icon_name="window-close-symbolic", tooltip_text="Close panel", halign=Gtk.Align.END, hexpand=True)
        close.add_css_class("flat")
        close.connect("clicked", lambda _b: self.split.set_show_sidebar(False))
        folder = Gtk.Button(icon_name="folder-open-symbolic", tooltip_text="Open the wallpaper's folder")
        folder.connect("clicked", lambda _b: Gio.AppInfo.launch_default_for_uri(it.dir.as_uri(), None))
        wid = it.workshop_id
        share = Gtk.Button(icon_name="send-to-symbolic", sensitive=bool(wid), tooltip_text="Share: copy the Steam Workshop link"
                           if wid else "Not on the Steam Workshop, so there is no link to share")
        share.connect("clicked", lambda _b: self._share(wid))
        tools.append(fav)
        tools.append(share)
        tools.append(folder)
        tools.append(reset)
        tools.append(close)
        box.append(tools)

        props = it.properties()
        if props:
            self.editor = PropertyEditor(props, self.edit_values, self._on_edit, self)
            box.append(self.editor)
        else:
            nl = Gtk.Label(label="This wallpaper has no adjustable settings.", xalign=0)
            nl.add_css_class("dim-label")
            box.append(nl)
        self.detail_scroll.set_child(box)
        self.split.set_show_sidebar(True)

    def _on_edit(self, key, value):
        self.dirty = True
        self.cfg.set_value(self.sel.id, key, value)
        # live re-apply (debounced) if this wallpaper is running somewhere
        if getattr(self, "_deb", None):
            GLib.source_remove(self._deb)
        self._deb = GLib.timeout_add(900, self._live_apply)

    def _live_apply(self):
        self._deb = None
        self.cfg.save()
        outs = [o for o, v in self.cfg.data["outputs"].items() if v.get("id") == self.sel.id and engine.running(o)]
        if outs:
            self.apply(outs, quiet=True)
        return False

    def _on_volume(self, value: int):
        if value >= 100:
            self.cfg.data["volumes"].pop(self.sel.id, None)
        else:
            self.cfg.data["volumes"][self.sel.id] = value
        if getattr(self, "_deb", None):
            GLib.source_remove(self._deb)
        self._deb = GLib.timeout_add(900, self._live_apply)

    def _share(self, workshop_id: str):
        """Copy the wallpaper's Steam Workshop page link; anyone can open it, or paste it into Discover here."""
        url = workshop.item_url(workshop_id)
        self.get_clipboard().set(url)
        t = Adw.Toast(title="Workshop link copied", button_label="Open page", timeout=6)
        t.connect("button-clicked", lambda _t: Gio.AppInfo.launch_default_for_uri(url, None))
        self.toasts.add_toast(t)

    def _toggle_fav(self, btn):
        fav = self.cfg.data["favorites"]
        if btn.get_active() and self.sel.id not in fav:
            fav.append(self.sel.id)
        elif not btn.get_active() and self.sel.id in fav:
            fav.remove(self.sel.id)
        self.cfg.save()

    def _reset(self):
        self.cfg.reset_values(self.sel.id)
        self.cfg.save()
        self.select(self.sel)
        self.toasts.add_toast(Adw.Toast(title="Settings reset to the wallpaper's defaults"))

    # ------------------------------------------------------------------ apply
    def apply(self, outputs: list[str], quiet=False, it: library.Item | None = None, sync=True):
        it = it or self.sel
        if not it or it.broken:
            self.toasts.add_toast(Adw.Toast(title=it.broken if it else "Nothing selected"))
            return
        mons = {o["name"]: o for o in monitors.outputs()}
        if it.ultrawide and not self.cfg.data["ultrawide_on_16_9"]:
            skipped = [o for o in outputs if not mons.get(o, {}).get("ultrawide", True)]
            if skipped:
                outputs = [o for o in outputs if o not in skipped]
                self.toasts.add_toast(Adw.Toast(title=f"Skipped {', '.join(skipped)}: ultrawide wallpaper on a 16:9 screen"))
                if not outputs:
                    return
        if it.kind == "web":
            if engine.web_missing():
                self.toasts.add_toast(Adw.Toast(title=f"Web wallpapers need {engine.web_missing()}: install it with your package manager",
                                                timeout=8))
                return
        elif not engine.use_native(self.cfg, it):
            if it.kind != "video" and not assets.have_assets(self.cfg.data["assets_dir"]):
                self._check_setup()
                self.toasts.add_toast(Adw.Toast(title="Scene wallpapers need Wallpaper Engine's assets (Settings > Setup)"))
                return
            if not paths.find_render() and not paths.find_engine(self.cfg.data["engine_dir"]):
                self._check_setup()
                if getattr(self, "_building", False):
                    msg = "The renderer is still building, try again in a minute"
                elif not shutil.which("cargo"):
                    msg = "Install Rust and clang first (Arch: sudo pacman -S rust clang), then restart WallForge"
                else:
                    msg = "The renderer is not built: see the banner at the top"
                self.toasts.add_toast(Adw.Toast(title=msg))
                return
        self.cfg.save()

        def work():
            err = ""
            try:
                for o in outputs:
                    engine.start(self.cfg, it, o)
                    self.cfg.data["outputs"][o] = {"id": it.id}
                    self._remember(o, it.id)
                self.cfg.save()
                if sync and outputs:
                    theme.sync(self.cfg, it, outputs[0] if len(outputs) == 1 else None)
            except Exception as e:
                err = str(e)
            GLib.idle_add(self._applied, it, outputs, quiet, err)
        threading.Thread(target=work, daemon=True).start()

    def _applied(self, it, outputs, quiet, err=""):
        if err:
            self.toasts.add_toast(Adw.Toast(title=err[:100]))
            return False
        if not quiet:
            self.toasts.add_toast(Adw.Toast(title=f"{it.title} on {', '.join(outputs)}"))
        self.refresh_playing()
        GLib.timeout_add_seconds(3, self._check_crash, outputs)
        return False

    def _check_crash(self, outputs):
        for o in outputs:
            if not engine.running(o) and engine.was_native(o) and self._fallback_legacy(o):
                continue
            if not engine.running(o):
                tail = engine.log_tail(o, 6).strip().splitlines()
                msg = tail[-1] if tail else "engine exited"
                t = Adw.Toast(title=f"{o}: engine stopped. {msg[:90]}", button_label="View log", timeout=10)
                t.connect("button-clicked", lambda _t, o=o: self._show_log(o))
                self.toasts.add_toast(t)
        self.refresh_playing()
        return False

    def _fallback_legacy(self, o) -> bool:
        """The native renderer died: retry once with linux-wallpaperengine if it is set up."""
        it = next((i for i in library.scan() if i.id == self.cfg.data["outputs"].get(o, {}).get("id")), None)
        if not it or not assets.have_assets(self.cfg.data["assets_dir"]) or not paths.find_engine(self.cfg.data["engine_dir"]):
            return False
        try:
            engine.start(self.cfg, it, o, legacy=True)
        except Exception:
            return False
        self.toasts.add_toast(Adw.Toast(title=f"{o}: native renderer stopped, switched to the classic engine"))
        GLib.timeout_add_seconds(3, self._check_crash, [o])
        return True

    def _periodic_update(self):
        if self.cfg.data["check_updates"]:
            self.check_update(quiet=True)
        return True

    def check_update(self, quiet=False):
        def work():
            try:
                res = updater.check()
                err = ""
            except Exception as e:
                res, err = None, str(e)
            GLib.idle_add(self._update_result, res, err, quiet)
        threading.Thread(target=work, daemon=True).start()

    def _update_result(self, res, err, quiet):
        if err:
            if not quiet:
                self.toasts.add_toast(Adw.Toast(title="Could not check for updates"))
            return False
        self.cfg.data["last_update_check"] = time.time()
        self.cfg.save()
        if not res["available"]:
            if not quiet:
                self.toasts.add_toast(Adw.Toast(title="WallForge is up to date"))
            return False
        if res["kind"] == "git" and self.cfg.data["auto_update"]:
            self._do_update(auto=True)
            return False
        if res["kind"] == "git":
            t = Adw.Toast(title="A WallForge update is available", button_label="Update", timeout=0)
            t.connect("button-clicked", lambda _t: self._do_update())
        else:
            t = Adw.Toast(title="A WallForge update is available. Update it with your package manager.", timeout=10)
        self.toasts.add_toast(t)
        return False

    def _do_update(self, auto=False):
        if getattr(self, "_updating", False):
            return
        self._updating = True

        def work():
            try:
                msg, err = updater.update(), ""
            except Exception as e:
                msg, err = "", str(e)
            GLib.idle_add(self._update_done, msg, err, auto)
        threading.Thread(target=work, daemon=True).start()

    def _update_done(self, msg, err, auto):
        self._updating = False
        if err:
            if not auto or "did not load" in err:
                self.toasts.add_toast(Adw.Toast(title=err[:100], timeout=8))
            return False
        t = Adw.Toast(title=f"WallForge updated ({msg[:50]})", button_label="Restart now", timeout=0)
        t.connect("button-clicked", lambda _t: updater.restart())
        self.toasts.add_toast(t)
        return False

    def _show_log(self, output):
        d = Adw.AlertDialog(heading=f"Engine log: {output}", body="Last lines of the engine output.")
        tv = Gtk.TextView(editable=False, monospace=True, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        tv.get_buffer().set_text(engine.log_tail(output, 30) or "(empty)")
        d.set_extra_child(Gtk.ScrolledWindow(child=tv, min_content_height=240, min_content_width=520))
        d.add_response("close", "Close")
        d.present(self)

    def random_wallpaper(self):
        pool = [i for i in self.items if not i.broken and not self._hidden_reason(i) and (i.kind != "web" or not engine.web_missing())]
        if not pool:
            self.toasts.add_toast(Adw.Toast(title="No wallpapers to pick from"))
            return
        self.sel = random.choice(pool)
        mons = self._mons or monitors.outputs()
        self.select(self.sel)
        self.apply([o["name"] for o in mons])

    # ------------------------------------------------------------------ rotation
    def _rotate_pool(self) -> list:
        """Wallpapers the automatic rotation may pick (same filters as the library view)."""
        return rotation.candidates(
            self.items,
            hidden=self._hidden_reason,
            favourites=self.cfg.data.get("favorites") or (),
            only_favourites=bool(self.cfg.data.get("rotate_favorites")),
            web_missing=bool(engine.web_missing()),
        )

    def _next_rotated(self, skip=()):
        """The wallpaper a timer tick should switch to, avoiding what is already showing."""
        cur = {v.get("id") for v in self.cfg.data["outputs"].values() if v.get("id")}
        return rotation.next_item(self._rotate_pool(), cur | set(skip),
                                  shuffle=bool(self.cfg.data.get("rotate_shuffle", True)))

    def _rotate_step(self):
        """Switch the screens to new wallpapers, one pick per screen unless that is turned off."""
        pool = self._rotate_pool()
        if not pool:
            return
        names = [o["name"] for o in (self._mons or monitors.outputs())]
        outs = [o for o in names if self.cfg.data["outputs"].get(o, {}).get("id") and o not in self._stopped]
        if not outs:
            return
        shuffle = bool(self.cfg.data.get("rotate_shuffle", True))
        if not self.cfg.data.get("rotate_per_output", True):
            it = rotation.next_item(pool, (), shuffle=shuffle)
            if it:
                self.sel = it
                try:
                    self.select(it)
                except Exception:
                    pass
                self.apply(outs, it=it)
            return
        showing = {self.cfg.data["outputs"].get(o, {}).get("id") for o in names}
        used: set[str] = set()
        first = None
        for out in outs:
            cur = self.cfg.data["outputs"].get(out, {}).get("id")
            avoid = (showing - {cur}) | used
            cand = [i for i in pool if i.id not in avoid] or [i for i in pool if i.id != cur] or pool
            it = rotation.next_item(cand, {cur} if cur else (), shuffle=shuffle)
            if not it:
                continue
            used.add(it.id)
            self.apply([out], it=it, sync=(first is None))
            if first is None:
                first = it
        if first is not None:
            self.sel = first
            try:
                self.select(first)
            except Exception:
                pass

    def _maybe_rotate(self):
        """Settings timer: change every wallpaper once the configured interval has passed."""
        mins = int(self.cfg.data.get("rotate_minutes") or 0)
        if mins <= 0 or engine.pause_for_battery(self.cfg):
            return
        now = time.time()
        if now - self._last_rotate < mins * 60:
            return
        self._last_rotate = now
        self.cfg.data["last_rotate"] = now
        self.cfg.save()
        self._rotate_step()

    def _rotation_changed(self):
        app = self.get_application()
        if app is not None:
            try:
                app._sync_hold(self)
            except Exception:
                pass

    def rotate_now(self):
        """Shortcut: switch to the next wallpaper(s) without waiting for the timer."""
        self._last_rotate = time.time()
        self.cfg.data["last_rotate"] = self._last_rotate
        self.cfg.save()
        self._rotate_step()

    def _shortcuts(self):
        ctl = Gtk.ShortcutController()
        ctl.set_scope(Gtk.ShortcutScope.GLOBAL)

        def add(trigger, fn):
            ctl.add_shortcut(Gtk.Shortcut.new(Gtk.ShortcutTrigger.parse_string(trigger),
                                              Gtk.CallbackAction.new(lambda *_: (fn(), True)[1])))
        add("<Control>f", lambda: (self.stack.set_visible_child_name("library"), self.search.grab_focus()))
        add("<Control>r", self.random_wallpaper)
        add("<Control><Shift>n", self.rotate_now)
        add("<Control>comma", self.show_settings)
        add("F5", lambda: self.reload_library(toast=True))
        add("Escape", lambda: self.split.set_show_sidebar(False))
        add("<Control>q", lambda: self.get_application().quit())
        self.add_controller(ctl)

    def _on_close(self, *_):
        self.cfg.data["window"] = {"w": self.get_width(), "h": self.get_height()}
        self.cfg.save()
        return False

    def _check_setup(self):
        """Videos need only the native renderer. Scenes also need Wallpaper Engine's assets (from your own copy).
        The classic linux-wallpaperengine is never required."""
        miss = []
        if not paths.find_render() and not paths.find_engine(self.cfg.data["engine_dir"]):
            if getattr(self, "_building", False):
                self.banner.set_title("Building the WallForge renderer, this takes a minute or two...")
                self.banner.set_revealed(True)
                return
            if not shutil.which("cargo"):
                self.banner.set_title("WallForge needs Rust to build its renderer: install rust and clang (Arch: sudo pacman -S rust clang), then restart.")
                self.banner.set_revealed(True)
                return
            if updater.BUILD_ERROR:
                self.banner.set_title("The renderer failed to build (" + updater.BUILD_ERROR + "). Install rust, clang, ffmpeg and libva, then restart.")
                self.banner.set_revealed(True)
                return
            miss.append("the WallForge renderer")
        elif any(i.kind == "scene" and not i.broken for i in self.items) and not assets.have_assets(self.cfg.data["assets_dir"]):
            miss.append("Wallpaper Engine's assets (needed for scene wallpapers)")
        self.banner.set_title("Setup needed: " + " and ".join(miss) + " missing." if miss else "")
        self.banner.set_revealed(bool(miss))

    def _build_playing_bar(self):
        self.play_box = Gtk.Box(spacing=16, margin_top=6, margin_bottom=6, margin_start=12, margin_end=12)
        return self.play_box

    def _poll_playing(self):
        try:
            self._watchdog()
        except Exception:
            pass
        try:
            self._maybe_rotate()
        except Exception:
            pass
        self.refresh_playing()
        return True

    def _remember(self, output: str, item_id: str):
        """Track what we started on an output so the watchdog can restart it."""
        self._applied[output] = item_id
        self._stopped.discard(output)
        self._restarts.pop(output, None)
        self._crashed.discard(output)
        self._started[output] = time.time()

    def _forget(self, output: str):
        """Explicit stop: no restart for this output."""
        self._applied.pop(output, None)
        self._stopped.add(output)
        self._restarts.pop(output, None)
        self._crashed.discard(output)
        self._started.pop(output, None)

    def stop_output(self, output: str):
        self._forget(output)
        try:
            engine.stop(output)
        except Exception:
            pass

    def _watchdog(self):
        """Restart wallpapers whose player died, unless battery saving says otherwise."""
        if engine.pause_for_battery(self.cfg):
            if not self._battery_paused:
                self._battery_paused = True
                for o in list(self._applied):
                    try:
                        engine.stop(o)
                    except Exception:
                        pass
            return
        self._battery_paused = False
        if not self._applied:
            return
        by_id = {i.id: i for i in self.items}
        now = time.time()
        for output, iid in list(self._applied.items()):
            if engine.running(output):
                self._crashed.discard(output)
                continue
            if output in self._crashed or now - self._started.get(output, 0) < 8:
                continue  # give a fresh start time to come up
            it = by_id.get(iid)
            if not it or it.broken:
                continue
            hist = [t for t in self._restarts.get(output, []) if now - t < 300]
            self._restarts[output] = hist
            if len(hist) >= 3:
                self._crashed.add(output)
                self.toasts.add_toast(Adw.Toast(title=f"{output}: stopped after repeated crashes"))
                continue
            try:
                engine.start(self.cfg, it, output)
                hist.append(now)
                self._started[output] = now
            except Exception:
                self._crashed.add(output)
                self.toasts.add_toast(Adw.Toast(title=f"{output}: could not restart the wallpaper"))

    def refresh_playing(self):
        # The 3 s poll calls this constantly; skip the widget churn when nothing actually changed.
        state = tuple((o["name"], engine.running(o["name"]), self.cfg.data["outputs"].get(o["name"], {}).get("id"),
                       o["name"] in self._crashed) for o in monitors.outputs()) + (self.cfg.data.get("rotate_minutes") or 0,)
        if state == getattr(self, "_play_state", None):
            return
        self._play_state = state
        child = self.play_box.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.play_box.remove(child)
            child = nxt
        by_id = {i.id: i for i in self.items}
        for o in monitors.outputs():
            name = o["name"]
            pid = engine.running(name)
            iid = self.cfg.data["outputs"].get(name, {}).get("id")
            b = Gtk.Box(spacing=6)
            ic = Gtk.Image(icon_name="video-display-symbolic")
            b.append(ic)
            if pid:
                title = by_id[iid].title if iid in by_id else "(external)"
                lab = Gtk.Label(label=f"{name}: {title}", ellipsize=3, max_width_chars=34)
                stop = Gtk.Button(icon_name="media-playback-stop-symbolic", tooltip_text="Stop", valign=Gtk.Align.CENTER)
                stop.add_css_class("flat")
                stop.connect("clicked", lambda _b, n=name: (self.stop_output(n), self.refresh_playing()))
                b.append(lab)
                b.append(stop)
            elif name in self._crashed:
                lab = Gtk.Label(label=f"{name}: stopped after repeated crashes")
                lab.add_css_class("dim-label")
                b.append(lab)
            else:
                lab = Gtk.Label(label=f"{name}: idle")
                lab.add_css_class("dim-label")
                b.append(lab)
            self.play_box.append(b)
        mins = int(self.cfg.data.get("rotate_minutes") or 0)
        if mins > 0:
            how = "a random one" if self.cfg.data.get("rotate_shuffle", True) else "the next one"
            note = Gtk.Label(label=f"rotating to {how} every {mins} min · Ctrl+Shift+N for next")
            note.add_css_class("dim-label")
            self.play_box.append(note)

    # ------------------------------------------------------------------ discover
    def _build_discover(self):
        page = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        bar = Gtk.Box(spacing=8, margin_top=8, margin_bottom=8, margin_start=12, margin_end=12)
        self.w_search = Gtk.SearchEntry(placeholder_text="Search the Steam Workshop, or paste a Workshop link", hexpand=True)
        self.w_search.connect("activate", lambda *_: self.web_search(1))
        self.w_sort = Gtk.DropDown.new_from_strings(["Trending", "Most recent", "Most subscribed"])
        self.w_type = Gtk.DropDown.new_from_strings(["Any type", "Scene", "Video", "Web"])
        for dd in (self.w_sort, self.w_type):
            dd.connect("notify::selected", lambda *_: self.web_search(1))
        self.w_prev = Gtk.Button(icon_name="go-previous-symbolic")
        self.w_next = Gtk.Button(icon_name="go-next-symbolic")
        self.w_prev.connect("clicked", lambda _b: self.web_search(self.web_page - 1))
        self.w_next.connect("clicked", lambda _b: self.web_search(self.web_page + 1))
        self.w_label = Gtk.Label(label="")
        self.w_label.add_css_class("dim-label")
        for w in (self.w_search, self.w_sort, self.w_type, self.w_prev, self.w_label, self.w_next):
            bar.append(w)
        page.append(bar)
        self.w_spin = Gtk.Spinner(spinning=False, halign=Gtk.Align.CENTER)
        page.append(self.w_spin)
        self.w_flow = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, homogeneous=True, min_children_per_line=2,
                                  max_children_per_line=8, column_spacing=12, row_spacing=12, margin_start=12,
                                  margin_end=12, margin_bottom=12, valign=Gtk.Align.START)
        page.append(Gtk.ScrolledWindow(vexpand=True, child=self.w_flow))
        self._first_discover = True
        self.stack.connect("notify::visible-child-name", self._on_tab)
        return page

    def _on_tab(self, *_):
        if self.stack.get_visible_child_name() == "discover" and self._first_discover:
            self._first_discover = False
            self.web_search(1)

    def web_search(self, page: int):
        if page < 1:
            return
        self.web_page = page
        sort = ["trend", "mostrecent", "totaluniquesubscribers"][self.w_sort.get_selected()]
        t = self.w_type.get_selected_item().get_string()
        tags = [] if t == "Any type" else [t]
        q = self.w_search.get_text().strip()
        wid = workshop.parse_item_id(q)
        if wid:
            self._open_link(wid)
            return
        exclude = []
        if not self.cfg.data["show_anime"]:
            exclude += ["Anime", "Girls"]
        if self.cfg.data["safe_content"]:
            exclude += workshop.MATURE_TAGS
        if not self.cfg.data["ultrawide_on_16_9"] and not self._has_ultrawide_monitor():
            exclude += workshop.ULTRAWIDE_TAGS
        self.w_spin.start()

        def work():
            try:
                res = workshop.browse(q, sort, page, tags, exclude=exclude)
                GLib.idle_add(self._show_web, res)
            except Exception as e:
                GLib.idle_add(self._web_error, str(e))
        threading.Thread(target=work, daemon=True).start()

    def _open_link(self, wid: str):
        """A pasted Workshop link or id: show that wallpaper (in the library when it is already installed)."""
        have = next((i for i in self.items if i.workshop_id == wid), None)
        if have:
            self.w_search.set_text("")
            self.stack.set_visible_child_name("library")
            self.select(have)
            self.toasts.add_toast(Adw.Toast(title="Already in your library"))
            return
        self.w_spin.start()

        def work():
            try:
                r = workshop.lookup(wid)
            except Exception as e:
                GLib.idle_add(self._web_error, str(e))
                return
            if r:
                GLib.idle_add(self._show_web, {"total": 1, "pages": 1, "items": [r]})
            else:
                GLib.idle_add(self._link_missing)
        threading.Thread(target=work, daemon=True).start()

    def _link_missing(self):
        self.w_spin.stop()
        self.toasts.add_toast(Adw.Toast(title="That link is not a Wallpaper Engine wallpaper, or it was removed"))
        return False

    def _web_error(self, msg):
        self.w_spin.stop()
        self.toasts.add_toast(Adw.Toast(title=f"Workshop unavailable: {msg[:80]}"))
        return False

    def _show_web(self, res):
        self.w_spin.stop()
        child = self.w_flow.get_first_child()
        while child:
            nxt = child.get_next_sibling()
            self.w_flow.remove(child)
            child = nxt
        self.w_label.set_text(f"page {self.web_page} of {res.get('pages') or '?'}")
        self.w_prev.set_sensitive(self.web_page > 1)
        self.w_next.set_sensitive(bool(res["pages"]) and self.web_page < res["pages"])
        for r in res["items"]:
            stars = f"{r['stars']:.1f}*" if isinstance(r.get("stars"), (int, float)) and r["stars"] else ""
            size = f"{r['size'] / 1e6:.0f} MB" if r["size"] else ""
            card = make_card(r["title"], r["preview"], r["type"] or "", "  ".join(x for x in (stars, size) if x))
            inner = card.get_last_child()
            btn = Gtk.Button()
            self._style_get(btn, r["id"])
            btn.connect("clicked", self._on_get, r)
            btn.set_hexpand(True)
            link = Gtk.Button(icon_name="send-to-symbolic", tooltip_text="Share: copy the Steam Workshop link")
            link.add_css_class("flat")
            link.connect("clicked", lambda _b, i=r["id"]: self._share(i))
            row = Gtk.Box(spacing=6)
            row.append(btn)
            row.append(link)
            inner.append(row)
            self.w_flow.append(card)
        return False

    def _style_get(self, btn, item_id):
        if workshop.installed(item_id):
            btn.set_label("Installed")
            btn.set_sensitive(False)
        else:
            btn.set_label("Get")
            btn.add_css_class("suggested-action")

    def _on_get(self, btn, r):
        btn.set_sensitive(False)
        btn.set_label("Subscribing...")

        def work():
            st = workshop.subscribe(r["id"])
            GLib.idle_add(self._after_subscribe, btn, r, st)
        threading.Thread(target=work, daemon=True).start()

    def _after_subscribe(self, btn, r, st):
        if st in ("clicked", "already"):
            btn.set_label("Downloading...")
            self.toasts.add_toast(Adw.Toast(title=f"Steam is downloading {r['title'][:40]}"))
        else:
            btn.set_label("Click Subscribe in Steam")
            btn.set_sensitive(True)
            self.toasts.add_toast(Adw.Toast(title="Steam opened the page. Click Subscribe there."))
        self._pending = getattr(self, "_pending", {})
        self._pending[r["id"]] = btn
        return False

    def _watch(self):
        """New Workshop folders (from Discover or Steam itself) appear in the library automatically."""
        try:
            now = {d.name for root in paths.workshop_dirs() for d in root.iterdir() if (d / "project.json").is_file()}
        except Exception:
            return True
        have = {i.id for i in self.items if any(i.dir.is_relative_to(r) for r in paths.workshop_dirs())}
        if now != have:
            self.reload_library()
            for i in now - have:
                b = getattr(self, "_pending", {}).pop(i, None)
                if b:
                    b.set_label("Installed")
                    b.set_sensitive(False)
                    b.remove_css_class("suggested-action")
                self.toasts.add_toast(Adw.Toast(title="New wallpaper installed"))
        return True

    # ------------------------------------------------------------------ settings
    def show_settings(self):
        d = Adw.PreferencesDialog(title="Settings")
        pg = Adw.PreferencesPage()
        d.add(pg)
        e = self.cfg.data["engine"]
        c = self.cfg.data

        # --- setup
        gs = Adw.PreferencesGroup(title="Setup", description="Video wallpapers need nothing else. Scene and web wallpapers still need the classic engine and "
                                  "Wallpaper Engine's assets until the native renderer covers them. You must own Wallpaper Engine on Steam, "
                                  "but you do not have to install it.")
        pg.add(gs)
        eng = paths.find_engine(c["engine_dir"])
        r_eng = Adw.ActionRow(title="Classic engine (linux-wallpaperengine), for scenes", subtitle=str(eng) if eng else
                              "Not found. Only needed for scene and web wallpapers. Install linux-wallpaperengine from your package manager or the AUR.")
        r_eng.add_prefix(Gtk.Image(icon_name="emblem-ok-symbolic" if eng else "dialog-warning-symbolic"))
        gs.add(r_eng)
        rn = paths.find_render()
        r_nat = Adw.SwitchRow(title="Native renderer (beta)", active=c.get("renderer", "native") == "native",
                              subtitle="On by default. Plays video wallpapers with WallForge's own Rust renderer (hardware decode, no extra "
                              "engine needed). Scene wallpapers still use linux-wallpaperengine." +
                              (f" Found: {rn}" if rn else " Not built yet."))
        r_nat.set_sensitive(bool(rn))
        r_nat.connect("notify::active", lambda rr, _p: (c.update(renderer="native" if rr.get_active() else "legacy"), self.cfg.save()))
        gs.add(r_nat)
        src = paths.render_source()
        if not rn and src and shutil.which("cargo"):
            r_b = Adw.ActionRow(title="Build the native renderer", subtitle="Runs cargo build --release (a few minutes the first time).")
            b_b = Gtk.Button(label="Build", valign=Gtk.Align.CENTER)

            def do_build(btn):
                import subprocess
                btn.set_sensitive(False)
                btn.set_label("Building...")

                def work():
                    r = subprocess.run(["cargo", "build", "--release"], cwd=str(src), capture_output=True, text=True)
                    ok = r.returncode == 0 and paths.find_render() is not None
                    err = (r.stderr.strip().splitlines() or ["Build failed"])[-1][:120]

                    def done():
                        btn.set_label("Done, reopen Settings" if ok else "Failed")
                        self.toasts.add_toast(Adw.Toast(title="Native renderer built" if ok else "Build failed: " + err))
                        return False
                    GLib.idle_add(done)
                threading.Thread(target=work, daemon=True).start()
            b_b.connect("clicked", do_build)
            r_b.add_suffix(b_b)
            gs.add(r_b)
        ap = paths.find_assets(c["assets_dir"])
        r_as = Adw.ActionRow(title="Wallpaper Engine assets", subtitle=str(ap) if ap else "Not found")
        r_as.add_prefix(Gtk.Image(icon_name="emblem-ok-symbolic" if ap else "dialog-warning-symbolic"))
        gs.add(r_as)
        if not ap:
            r_dl = Adw.ActionRow(title="Fetch assets from Steam (about 85 MB)",
                                 subtitle="Opens a terminal where you sign in to Steam. Needs an account that owns Wallpaper Engine.")
            b = Gtk.Button(label="Fetch", valign=Gtk.Align.CENTER)
            b.add_css_class("suggested-action")
            b.connect("clicked", lambda btn: self._fetch_assets(btn))
            r_dl.add_suffix(b)
            gs.add(r_dl)
            r_st = Adw.ActionRow(title="Or install through Steam instead", subtitle="Installs the full game (about 0.8 GB). WallForge then copies the assets.")
            b2 = Gtk.Button(label="Open Steam", valign=Gtk.Align.CENTER)
            b2.connect("clicked", lambda _b: (Gio.AppInfo.launch_default_for_uri(assets.steam_install_uri(), None),
                                              GLib.timeout_add_seconds(10, self._poll_steam_install)))
            r_st.add_suffix(b2)
            gs.add(r_st)
            r_re = Adw.ActionRow(title="Check again")
            b3 = Gtk.Button(label="Re-check", valign=Gtk.Align.CENTER)
            b3.connect("clicked", lambda _b: (self._check_setup(), d.close(), self.show_settings()))
            r_re.add_suffix(b3)
            gs.add(r_re)

        # --- content
        gc = Adw.PreferencesGroup(title="Content")
        pg.add(gc)

        def top_switch(group, title, key, sub="", after=None):
            r = Adw.SwitchRow(title=title, subtitle=sub, active=bool(c.get(key)))

            def ch(rr, _p):
                c[key] = rr.get_active()
                self.cfg.save()
                if after:
                    after()
            r.connect("notify::active", ch)
            group.add(r)

        def refresh_filters():
            self.flow.invalidate_filter()
            self._update_hidden()
            if not self._first_discover:
                self.web_search(1)
        top_switch(gc, "Show anime wallpapers", "show_anime", "Hidden by default in Library and Discover.", refresh_filters)
        top_switch(gc, "Hide mature content in Discover", "safe_content", "Filters Mature and Questionable Workshop items.", refresh_filters)
        top_switch(gc, "Allow ultrawide wallpapers on 16:9 screens", "ultrawide_on_16_9",
                   "On: 21:9 and dual-monitor wallpapers apply to 16:9 screens, cropped to fill. Off: they are hidden and skipped there.",
                   refresh_filters)

        # --- playback
        g1 = Adw.PreferencesGroup(title="Playback")
        pg.add(g1)

        def combo(title, key, vals, sub=""):
            r = Adw.ComboRow(title=title, subtitle=sub, model=Gtk.StringList.new(vals))
            if e.get(key) in vals:
                r.set_selected(vals.index(e[key]))
            r.connect("notify::selected", lambda rr, _p: self._set_e(key, vals[rr.get_selected()]))
            g1.add(r)

        def switch(title, key, sub=""):
            r = Adw.SwitchRow(title=title, subtitle=sub, active=bool(e.get(key)))
            r.connect("notify::active", lambda rr, _p: self._set_e(key, rr.get_active()))
            g1.add(r)

        def spin(title, key, lo, hi):
            r = Adw.SpinRow.new_with_range(lo, hi, 1)
            r.set_title(title)
            r.set_value(e.get(key, lo))
            r.connect("notify::value", lambda rr, _p: self._set_e(key, int(rr.get_value())))
            g1.add(r)
        spin("Frame rate limit", "fps", 5, 240)
        spin("Volume", "volume", 0, 100)
        switch("Mute", "silent")
        combo("Scaling", "scaling", ["auto", "fill", "fit", "stretch", "default"], "auto crops to fill the screen (ultrawide art included) and only fits portrait wallpapers; fill crops, fit shows all of it, stretch distorts")
        combo("Edge behaviour", "clamp", ["clamp", "border", "repeat"])
        combo("Layer", "layer", ["bottom", "background"])
        rot = Adw.SpinRow.new_with_range(0, 1440, 5)
        rot.set_title("Change wallpaper every (minutes, 0 = never)")
        rot.set_value(c.get("rotate_minutes", 0))
        def rot_changed(rr, _p):
            c["rotate_minutes"] = int(rr.get_value())
            self.cfg.save()
            self._rotation_changed()
        rot.connect("notify::value", rot_changed)
        rot.set_subtitle("While rotation is on, closing the window keeps WallForge running in the background so it can still change the wallpaper. Ctrl+Q quits completely.")
        g1.add(rot)
        top_switch(g1, "Pick a random wallpaper when it changes", "rotate_shuffle", "Off: walk the library in order.")
        top_switch(g1, "Rotate only through favourites", "rotate_favorites")
        top_switch(g1, "Give each screen its own wallpaper", "rotate_per_output",
                   "On: monitors rotate independently. Off: all screens change to the same wallpaper.")
        g2 = Adw.PreferencesGroup(title="Effects and power")
        pg.add(g2)
        g1 = g2
        switch("Disable particles", "disable_particles")
        spin("Render resolution (%)", "render_scale", 25, 100)
        switch("Reduce frame rate and resolution on battery", "reduce_on_battery")
        switch("Pause wallpapers on battery", "pause_on_battery")
        switch("Disable mouse interaction", "disable_mouse")
        switch("Disable parallax", "disable_parallax")
        switch("Keep playing under fullscreen apps", "no_fullscreen_pause")
        switch("Pause only for the active fullscreen window", "fullscreen_pause_only_active")

        # --- desktop
        g3 = Adw.PreferencesGroup(title="Desktop")
        pg.add(g3)
        found = theme.available()
        vals = theme.BACKENDS
        r = Adw.ComboRow(title="Sync colours with", subtitle="Found: " + (", ".join(found) if found else "no palette tool"),
                         model=Gtk.StringList.new(vals))
        r.set_selected(vals.index(c.get("palette_backend", "auto")) if c.get("palette_backend", "auto") in vals else 0)
        r.connect("notify::selected", lambda rr, _p: (c.update(palette_backend=vals[rr.get_selected()]), self.cfg.save()))
        g3.add(r)
        ce = Adw.EntryRow(title="Custom palette command ({image} is the wallpaper image)", text=c.get("palette_command", ""))
        ce.connect("changed", lambda er: (c.update(palette_command=er.get_text()), self.cfg.save()))
        g3.add(ce)
        r = Adw.SwitchRow(title="Restore wallpapers at login", subtitle="Writes an XDG autostart entry. If your compositor "
                          "ignores those, run wallforge.sh --restore from its startup config.", active=autostart_enabled())
        r.connect("notify::active", lambda rr, _p: set_autostart(rr.get_active()))
        g3.add(r)
        gw = Adw.PreferencesGroup(title="Wallpaper Engine", description="Bring over what you set up in Wallpaper Engine itself: "
                                  "each wallpaper's customised settings and volume, the frame rate limit and the fullscreen rule.")
        pg.add(gw)
        we_cfgs = weimport.find_configs()
        ri = Adw.ActionRow(title="Import settings", subtitle=str(we_cfgs[0]) if we_cfgs else
                           "Pick config.json from the wallpaper_engine folder of the install you used (for example on your Windows drive).")
        if we_cfgs:
            bi = Gtk.Button(label="Import", valign=Gtk.Align.CENTER)
            bi.connect("clicked", lambda _b: (d.close(), self._import_we(we_cfgs[0])))
            ri.add_suffix(bi)
        bf = Gtk.Button(label="Other file" if we_cfgs else "Choose file", valign=Gtk.Align.CENTER)
        bf.connect("clicked", lambda _b: (d.close(), self._pick_we()))
        ri.add_suffix(bf)
        gw.add(ri)
        gu = Adw.PreferencesGroup(title="Updates")
        pg.add(gu)
        top_switch(gu, "Check for updates", "check_updates", "At start and every few hours, asks GitHub whether a newer WallForge exists.")
        top_switch(gu, "Install updates automatically", "auto_update", "Git installs only: downloads the new version in the background, "
                   "checks it loads, and asks you to restart. Package installs update through your package manager.")
        ru = Adw.ActionRow(title="Check now", subtitle=f"Version {__version__}, installed as: {updater.kind()}")
        bu = Gtk.Button(label="Check", valign=Gtk.Align.CENTER)
        bu.connect("clicked", lambda _b: self.check_update())
        ru.add_suffix(bu)
        gu.add(ru)
        n = Adw.ActionRow(title="Changes apply the next time a wallpaper starts")
        n.add_css_class("dim-label")
        g3.add(n)
        d.present(self)

    def _pick_we(self):
        dlg = Gtk.FileDialog(title="Wallpaper Engine config.json")
        flt = Gtk.FileFilter(name="config.json")
        flt.add_pattern("*.json")
        store = Gio.ListStore.new(Gtk.FileFilter)
        store.append(flt)
        dlg.set_filters(store)

        def done(dl, res):
            try:
                f = dl.open_finish(res)
            except GLib.Error:
                return
            if f.get_path():
                self._import_we(Path(f.get_path()))
        dlg.open(self, None, done)

    def _import_we(self, path: Path):
        """Show what a Wallpaper Engine config.json would change, then import it if confirmed."""
        try:
            plan = weimport.plan(path, self.items)
        except (OSError, ValueError) as ex:
            self.toasts.add_toast(Adw.Toast(title=str(ex)[:110] or "Could not read that file"))
            return
        titles = {i.id: i.title for i in self.items}
        if plan.empty:
            self.toasts.add_toast(Adw.Toast(title=(plan.notes() or ["Nothing to import in that file"])[0][:110], timeout=8))
            return
        by_id = {i.id: i for i in self.items}
        todo = {o: by_id[i] for o, i in plan.assignments([m["name"] for m in (self._mons or monitors.outputs())]).items()
                if i in by_id and not by_id[i].broken}
        body = "\n".join(["From " + str(path), ""] + ["• " + s for s in plan.summary(titles)] + [""] + plan.notes()).strip()
        ad = Adw.AlertDialog(heading="Import from Wallpaper Engine?", body=body)
        ad.add_response("cancel", "Cancel")
        ad.add_response("import", "Import")
        if todo:
            ad.add_response("apply", "Import and set wallpaper")
        ad.set_response_appearance("import", Adw.ResponseAppearance.SUGGESTED)
        ad.set_default_response("import")
        ad.set_close_response("cancel")

        def chosen(_d, resp):
            if resp not in ("import", "apply"):
                return
            weimport.apply(self.cfg, plan)
            if resp == "apply":
                groups = {}
                for o, it in todo.items():
                    groups.setdefault(it.id, (it, []))[1].append(o)
                for it, outs in groups.values():
                    self.apply(outs, quiet=True, it=it)
            if self.sel and self.split.get_show_sidebar():
                self.select(self.sel)
            self.toasts.add_toast(Adw.Toast(title="Imported from Wallpaper Engine: " + "; ".join(plan.summary(titles))[:90], timeout=8))
        ad.connect("response", chosen)
        ad.present(self)

    def _fetch_assets(self, btn):
        btn.set_sensitive(False)
        btn.set_label("Preparing...")

        def work():
            err = ""
            try:
                if not assets.depot_downloader():
                    assets.install_depot_downloader()
                err = assets.launch_download()
            except Exception as ex:
                err = str(ex)
            GLib.idle_add(self._fetch_started, btn, err)
        threading.Thread(target=work, daemon=True).start()

    def _fetch_started(self, btn, err):
        if err:
            btn.set_sensitive(True)
            btn.set_label("Fetch")
            self.toasts.add_toast(Adw.Toast(title=err[:110]))
        else:
            btn.set_label("Waiting for terminal...")
            self.toasts.add_toast(Adw.Toast(title="Sign in to Steam in the terminal window"))
            GLib.timeout_add_seconds(3, self._poll_assets)
        return False

    def _poll_assets(self):
        if assets.have_assets(self.cfg.data["assets_dir"]):
            self._check_setup()
            self.toasts.add_toast(Adw.Toast(title="Assets ready. You can apply wallpapers now."))
            return False
        return True

    def _poll_steam_install(self):
        if paths.find_assets(self.cfg.data["assets_dir"]):
            self._check_setup()
            return False
        for d in paths.we_dirs():
            if (d / "assets" / "shaders").is_dir():
                if assets.copy_from_steam_install():
                    self._check_setup()
                    self.toasts.add_toast(Adw.Toast(title="Assets copied. Ready."))
                    return False
        return True

    def _set_e(self, key, val):
        self.cfg.data["engine"][key] = val
        self.cfg.save()


AUTOSTART = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "autostart/dev.wallforge.Wallforge-restore.desktop"


def autostart_enabled() -> bool:
    return AUTOSTART.is_file()


def set_autostart(on: bool):
    if not on:
        AUTOSTART.unlink(missing_ok=True)
        return
    launcher = shutil.which("wallforge") or str(Path(__file__).resolve().parent.parent / "wallforge.sh")
    AUTOSTART.parent.mkdir(parents=True, exist_ok=True)
    AUTOSTART.write_text(f"[Desktop Entry]\nType=Application\nName=WallForge (restore wallpapers)\n"
                         f"Exec={launcher} --restore\nIcon=dev.wallforge.Wallforge\nNoDisplay=true\n"
                         "X-GNOME-Autostart-enabled=true\n")


class _FlowFilter:
    def __init__(self, flow):
        self.flow = flow

    def changed(self, *_):
        self.flow.invalidate_filter()


class App(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID)
        self.cfg = config.Config()

    def do_activate(self):
        if not getattr(self, "_css", False):  # only once: reopening the window must not stack CSS providers
            prov = Gtk.CssProvider()
            prov.load_from_string(CSS)
            Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), prov, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
            Gtk.Window.set_default_icon_name(APP_ID)
            self._css = True
        win = self.props.active_window or Window(self, self.cfg)
        win.present()
        self._sync_hold(win)

    def _sync_hold(self, win=None):
        """Keep the process alive without a window while the rotation timer is on, so it keeps rotating."""
        cfg = win.cfg if win is not None else self.cfg
        want = int(cfg.data.get("rotate_minutes") or 0) > 0
        held = getattr(self, "_held", False)
        if want and not held:
            self.hold()
            self._held = True
        elif not want and held:
            self._held = False
            self.release()


def restore(cfg: config.Config) -> int:
    """Headless: start every saved assignment (for autostart)."""
    n = 0
    items = {i.id: i for i in library.scan()}
    for out, v in cfg.data["outputs"].items():
        it = items.get(v.get("id"))
        if it and not it.broken and not engine.running(out):
            try:
                engine.start(cfg, it, out)
                n += 1
            except Exception as e:
                print(f"{out}: {e}", file=sys.stderr)
    return n


def main():
    if "--restore" in sys.argv:
        print("started", restore(config.Config()))
        return 0
    if "--import-we" in sys.argv:  # --import-we [config.json] [--dry-run]
        return weimport.main(sys.argv[sys.argv.index("--import-we") + 1:])
    return App().run([sys.argv[0]])
