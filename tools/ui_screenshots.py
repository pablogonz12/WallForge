"""Screenshot the app off-screen, for the README.

    tools/ui_screenshots.sh OUT_PREFIX

Runs WallForge on GTK's Broadway backend (nothing appears on your desktop) with a scratch copy of your config whose
outputs and rotation are cleared, so it never starts or changes a wallpaper. Writes OUT_PREFIX-library.png and
OUT_PREFIX-settings.png.
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
import gi

gi.require_version("Gtk", "4.0"); gi.require_version("Adw", "1")
from gi.repository import GLib, Gtk

from wallforge import app as wfapp

out = sys.argv[1]
def render(win, path):
    w, h = win.get_width(), win.get_height()
    p = Gtk.WidgetPaintable.new(win); s = Gtk.Snapshot(); p.snapshot(s, w, h)
    node = s.to_node()
    win.get_native().get_renderer().render_texture(node, None).save_to_png(path)
    print("saved", path, w, h)

a = wfapp.App()
def step1():
    win = a.get_active_window()
    render(win, out + "-library.png")
    win.show_settings()
    GLib.timeout_add(4000, step2)
    return False
def step2():
    win = a.get_active_window()
    render(win, out + "-settings.png")
    a.quit(); return False
a.connect("activate", lambda *_: GLib.timeout_add(9000, step1))
a.run([sys.argv[0]])
