"""Web wallpaper player: a WebKit view on the wallpaper layer of one output.

  python3 -m wallforge.webplayer --output DP-2 --bg <wallpaper dir> [--layer bottom|background] [--fps N]
      [--silent | --volume N] [--mouse] [--set-property key=value ...]
      [--window WxH] [--screenshot FILE --screenshot-time S]      (testing: a normal window, then a PNG and exit)

Speaks the Wallpaper Engine web API the page expects: window.wallpaperPropertyListener.applyUserProperties /
applyGeneralProperties, and wallpaperRegisterAudioListener (fed silence: there is no audio capture yet).
Needs webkitgtk-6.0 and gtk4-layer-shell.
"""
from __future__ import annotations

import ctypes
import json
import signal
import sys
from pathlib import Path

LAYER_LIB = "libgtk4-layer-shell.so"
COMM = b"wallforge-web"  # process name, so engine.find_pids() can find the player for an output


def available() -> str:
    """Empty when web wallpapers can play here, else what is missing."""
    import gi
    miss = []
    for ns, ver, pkg in (("WebKit", "6.0", "webkitgtk-6.0"), ("Gtk4LayerShell", "1.0", "gtk4-layer-shell")):
        try:
            gi.require_version(ns, ver)
        except ValueError:
            miss.append(pkg)
    return ", ".join(miss)


def _args(argv):
    a = {"output": "", "bg": "", "layer": "bottom", "fps": 30, "volume": 100, "silent": False, "mouse": False, "props": {},
         "window": "", "screenshot": "", "screenshot_time": 2.0}
    it = iter(argv)
    for x in it:
        if x == "--silent":
            a["silent"] = True
        elif x == "--mouse":
            a["mouse"] = True
        elif x == "--set-property":
            k, _, v = next(it, "").partition("=")
            if k:
                a["props"][k] = v
        elif x.startswith("--") and x[2:].replace("-", "_") in a:
            key = x[2:].replace("-", "_")
            v = next(it, "")
            a[key] = type(a[key])(v) if not isinstance(a[key], bool) else True
        else:
            sys.exit(f"wallforge-web: unknown argument {x}")
    if not a["bg"]:
        sys.exit(__doc__)
    return a


def _typed(spec: dict, raw: str, base: Path):
    """A --set-property string as the JSON value the page expects for that property type."""
    t = str(spec.get("type", "")).lower()
    if t == "bool":
        return raw.strip().lower() in ("1", "true")
    if t == "slider":
        try:
            f = float(raw)
            return int(f) if f.is_integer() and "." not in raw else f
        except ValueError:
            return spec.get("value")
    if t in ("file", "directory", "scenetexture") and raw:
        p = Path(raw)
        return (p if p.is_absolute() else base / p).as_uri()
    if t == "combo":
        for o in spec.get("options") or []:
            if isinstance(o, dict) and str(o.get("value")) == raw:
                return o.get("value")
    return raw


def user_properties(project: dict, overrides: dict, base: Path) -> dict:
    """{key: {type, value, ...}} as Wallpaper Engine hands it to applyUserProperties."""
    out = {}
    for k, spec in ((project.get("general") or {}).get("properties") or {}).items():
        if not isinstance(spec, dict):
            continue
        p = {x: spec[x] for x in ("type", "value", "options", "min", "max", "step") if x in spec}
        if k in overrides:
            p["value"] = _typed(spec, overrides[k], base)
        out[k] = p
    return out


BOOT = r"""
(function () {
  if (window.__wf) return;
  var W = window.__wf = {props: %(props)s, general: %(general)s, volume: %(volume)s, audio: null};
  function user() { try { var l = window.wallpaperPropertyListener;
    if (l && l.applyGeneralProperties) l.applyGeneralProperties(W.general);
    if (l && l.applyUserProperties) l.applyUserProperties(W.props); } catch (e) { console.error("wallforge: " + e); } }
  var cur;
  try { Object.defineProperty(window, "wallpaperPropertyListener", {configurable: true,
    get: function () { return cur; }, set: function (v) { cur = v; setTimeout(user, 0); }}); } catch (e) {}
  window.addEventListener("load", function () { setTimeout(user, 0); vol(); });
  window.wallpaperRegisterAudioListener = function (cb) { W.audio = cb;
    if (!W.timer && cb) { var z = new Array(128).fill(0);
      W.timer = setInterval(function () { try { W.audio(z); } catch (e) {} }, 100); } };
  window.wallpaperRegisterMediaStatusListener = window.wallpaperRegisterMediaPropertiesListener =
  window.wallpaperRegisterMediaThumbnailListener = window.wallpaperRegisterMediaPlaybackListener =
  window.wallpaperRegisterMediaTimelineListener = function () {};
  window.wallpaperRequestRandomFileForProperty = function (name, cb) { if (cb) cb(name, ""); };
  function vol() { if (W.volume >= 1) return;
    document.querySelectorAll("audio,video").forEach(function (m) { m.volume = Math.min(m.volume, W.volume); }); }
  if (W.volume < 1) setInterval(vol, 2000);
})();
"""


def main(argv=None) -> int:
    a = _args(sys.argv[1:] if argv is None else argv)
    windowed = bool(a["window"] or a["screenshot"])
    if not windowed:
        try:
            ctypes.CDLL(LAYER_LIB)  # must be loaded before GTK connects to the compositor
        except OSError:
            sys.exit("wallforge-web: gtk4-layer-shell is not installed")
    try:
        ctypes.CDLL(None).prctl(15, COMM, 0, 0, 0)  # PR_SET_NAME
    except Exception:
        pass
    import gi
    gi.require_version("Gdk", "4.0")
    gi.require_version("Gtk", "4.0")
    gi.require_version("WebKit", "6.0")
    from gi.repository import Gdk, Gio, GLib, Gtk, WebKit
    if not windowed:
        gi.require_version("Gtk4LayerShell", "1.0")
        from gi.repository import Gtk4LayerShell as LS

    root = Path(a["bg"]).resolve()
    try:
        with open(root / "project.json", encoding="utf-8-sig") as fh:
            project = json.load(fh)
    except (OSError, ValueError) as e:
        sys.exit(f"wallforge-web: {root}/project.json: {e}")
    page = root / str(project.get("file") or "index.html")
    if str(project.get("type", "")).lower() != "web" or not page.is_file():
        sys.exit(f"wallforge-web: not a web wallpaper: {root}")
    boot = BOOT % {"props": json.dumps(user_properties(project, a["props"], root)),
                   "general": json.dumps({"fps": a["fps"]}), "volume": 0 if a["silent"] else a["volume"] / 100}

    app = Gtk.Application(application_id=None, flags=Gio.ApplicationFlags.NON_UNIQUE)

    def activate(_app):
        win = Gtk.ApplicationWindow(application=app, title="WallForge web wallpaper", decorated=False)
        if windowed:
            w, _, h = (a["window"] or "960x540").partition("x")
            win.set_default_size(int(w), int(h))
        else:
            LS.init_for_window(win)
            LS.set_namespace(win, "wallforge")
            LS.set_layer(win, LS.Layer.BACKGROUND if a["layer"] == "background" else LS.Layer.BOTTOM)
            for edge in (LS.Edge.TOP, LS.Edge.BOTTOM, LS.Edge.LEFT, LS.Edge.RIGHT):
                LS.set_anchor(win, edge, True)
            LS.set_exclusive_zone(win, -1)
            LS.set_keyboard_mode(win, LS.KeyboardMode.NONE)
            mons = Gdk.Display.get_default().get_monitors()
            for i in range(mons.get_n_items()):
                m = mons.get_item(i)
                if m.get_connector() == a["output"]:
                    LS.set_monitor(win, m)
                    break
            else:
                if a["output"]:
                    sys.exit(f"wallforge-web: output {a['output']} not found")
        ucm = WebKit.UserContentManager()
        ucm.add_script(WebKit.UserScript.new(boot, WebKit.UserContentInjectedFrames.ALL_FRAMES,
                                             WebKit.UserScriptInjectionTime.START, None, None))
        view = WebKit.WebView(user_content_manager=ucm)
        s = view.get_settings()
        for name, val in (("enable-webgl", True), ("allow-file-access-from-file-urls", True),
                          ("allow-universal-access-from-file-urls", True), ("media-playback-requires-user-gesture", False),
                          ("enable-write-console-messages-to-stdout", True), ("enable-developer-extras", False),
                          ("enable-back-forward-navigation-gestures", False)):
            try:
                s.set_property(name, val)
            except TypeError:
                pass
        black = Gdk.RGBA()
        black.parse("black")
        view.set_background_color(black)
        view.set_is_muted(bool(a["silent"]) or a["volume"] <= 0)
        view.connect("context-menu", lambda *_: True)  # no browser menu on the desktop
        view.connect("decide-policy", _stay_local)
        win.set_child(view)
        if not a["mouse"] and not windowed:
            import cairo
            win.connect("realize", lambda w: w.get_surface().set_input_region(cairo.Region()))
        view.load_uri(page.as_uri())
        win.present()
        if a["screenshot"]:
            def snap():
                def done(v, res):
                    try:
                        v.get_snapshot_finish(res).save_to_png(a["screenshot"])
                        print(f"wallforge-web: screenshot {a['screenshot']}")
                    except GLib.Error as e:
                        print(f"wallforge-web: screenshot failed: {e.message}", file=sys.stderr)
                    app.quit()
                view.get_snapshot(WebKit.SnapshotRegion.VISIBLE, WebKit.SnapshotOptions.NONE, None, done)
                return False
            GLib.timeout_add(int(a["screenshot_time"] * 1000), snap)

    def _stay_local(_view, decision, kind):
        # a wallpaper must not wander off to other pages in the top frame; sub-resources are unaffected
        if kind == WebKit.PolicyDecisionType.NAVIGATION_ACTION:
            uri = decision.get_navigation_action().get_request().get_uri() or ""
            if decision.get_navigation_action().is_user_gesture() and not uri.startswith("file://"):
                decision.ignore()
                return True
        return False

    app.connect("activate", activate)
    try:
        gi.require_version("GLibUnix", "2.0")
        from gi.repository import GLibUnix
        add_signal = GLibUnix.signal_add
    except (ValueError, ImportError):
        add_signal = GLib.unix_signal_add
    for sig in (signal.SIGTERM, signal.SIGINT):
        add_signal(GLib.PRIORITY_HIGH, sig, lambda: (app.quit(), False)[1])
    return app.run([sys.argv[0]])


if __name__ == "__main__":
    sys.exit(main())
