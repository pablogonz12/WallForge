"""Tests for the local/Lively importer and library scan integration (no GTK needed)."""
import json
import os
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from wallforge import importer, library, paths


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)
        self.lib = self.t / "lib"
        p = mock.patch.dict(os.environ, {"WALLFORGE_LIBRARY": str(self.lib)})
        p.start()
        self.addCleanup(p.stop)
        self.addCleanup(self.tmp.cleanup)
        for name in ("workshop_dirs", "we_dirs"):
            q = mock.patch.object(paths, name, return_value=[])
            q.start()
            self.addCleanup(q.stop)

    def zip(self, name, files):
        z = self.t / name
        with zipfile.ZipFile(z, "w") as zf:
            for n, data in files.items():
                zf.writestr(n, data)
        return z


class LivelyMapTest(Base):
    def test_string_and_numeric_types(self):
        for t, want in [("video", "video"), (7, "video"), ("gif", "video"), (8, "video"), ("web", "web"), (1, "web"),
                        ("webaudio", "web"), (2, "web"), ("picture", "web"), (100, "web"), ("7", "video")]:
            self.assertEqual(importer.map_lively({"Type": t, "FileName": "a.x", "Title": "T"})["type"], want, t)

    def test_unsupported_types_give_clear_error(self):
        for t in ("app", 0, "unity", 5, "godot", 6, "url", 3, "bizhawk", 4):
            with self.assertRaises(importer.ImportError_) as c:
                importer.map_lively({"Type": t, "FileName": "a", "Title": "X"})
            self.assertIn("can only run", str(c.exception))

    def test_lively_zip_import_and_scan(self):
        z = self.zip("v.zip", {"LivelyInfo.json": json.dumps({"Type": 7, "FileName": "clip.mp4", "Title": "Clip", "Author": "me"}),
                               "clip.mp4": b"not really video"})
        d = importer.import_path(z)
        self.assertTrue(d.name.startswith("local-"))
        items = library.scan()
        self.assertEqual([(i.id, i.kind, i.source, i.title) for i in items], [(d.name, "video", "Lively", "Clip")])
        self.assertFalse(items[0].broken)

    def test_unsupported_zip_leaves_nothing(self):
        z = self.zip("a.zip", {"LivelyInfo.json": json.dumps({"Type": "app", "FileName": "x.exe", "Title": "X"}), "x.exe": b"MZ"})
        with self.assertRaises(importer.ImportError_):
            importer.import_path(z)
        self.assertEqual(list(self.lib.iterdir()) if self.lib.exists() else [], [])

    def test_nested_folder_package(self):
        z = self.zip("n.zip", {"pkg/LivelyInfo.json": json.dumps({"Type": "web", "FileName": "index.html", "Title": "W"}),
                               "pkg/index.html": "<html>"})
        d = importer.import_path(z)
        self.assertEqual(json.loads((d / "project.json").read_text())["type"], "web")
        self.assertTrue((d / "index.html").is_file())


class ZipSlipTest(Base):
    def test_rejects_parent_and_absolute_paths(self):
        for bad in ("../evil.txt", "a/../../evil.txt", "/abs/evil.txt", "C:/evil.txt", "a\\..\\..\\evil.txt"):
            z = self.zip("bad.zip", {"LivelyInfo.json": json.dumps({"Type": "web", "FileName": "i.html", "Title": "T"}), bad: "x"})
            with self.assertRaises(importer.ImportError_, msg=bad):
                importer.import_path(z)
            self.assertFalse((self.t / "evil.txt").exists())
        self.assertEqual(list(self.lib.iterdir()), [])

    def test_size_cap(self):
        z = self.zip("big.zip", {"a": "x" * 1000})
        with zipfile.ZipFile(z) as zf, self.assertRaises(importer.ImportError_):
            importer.safe_extract(zf, self.t / "out", max_bytes=100)


class LocalImportTest(Base):
    def test_web_preview_generated_and_existing_preserved(self):
        w = self.t / "page"
        w.mkdir()
        (w / "other.html").write_text("<html>")
        def screenshot(d):
            self.assertEqual(json.loads((d / "project.json").read_text())["file"], "other.html")
            (d / "preview.jpg").write_bytes(b"jpeg")
            return "preview.jpg"
        with mock.patch.object(importer, "_web_thumb", side_effect=screenshot) as thumb:
            d = importer.import_path(w / "other.html")
            self.assertEqual(json.loads((d / "project.json").read_text())["preview"], "preview.jpg")
            (w / "preview.png").write_bytes(b"existing")
            d = importer.import_path(w / "other.html")
            self.assertEqual(json.loads((d / "project.json").read_text())["preview"], "preview.png")
            self.assertEqual(thumb.call_count, 1)

    def test_web_thumbnail_failure_is_nonfatal(self):
        w = self.t / "page"
        w.mkdir()
        (w / "index.html").write_text("<html>")
        with mock.patch.object(importer, "_web_thumb", return_value=None):
            d = importer.import_path(w)
        self.assertNotIn("preview", json.loads((d / "project.json").read_text()))

    def test_lively_web_thumbnail_fallback(self):
        for kind in ("web", "webaudio"):
            z = self.zip(f"{kind}.zip", {"LivelyInfo.json": json.dumps(
                {"Type": kind, "FileName": "page.html", "Thumbnail": "missing.png"}),
                "page.html": "<html>"})
            with mock.patch.object(importer, "_web_thumb", return_value="preview.jpg") as thumb:
                d = importer.import_path(z)
            self.assertEqual(json.loads((d / "project.json").read_text())["preview"], "preview.jpg")
            thumb.assert_called_once_with(d)

    def test_lively_existing_thumbnail_preserved(self):
        z = self.zip("existing.zip", {"LivelyInfo.json": json.dumps(
            {"Type": "web", "FileName": "page.html", "Thumbnail": "thumb.png"}),
            "page.html": "<html>", "thumb.png": b"existing"})
        with mock.patch.object(importer, "_web_thumb") as thumb:
            d = importer.import_path(z)
        self.assertEqual(json.loads((d / "project.json").read_text())["preview"], "thumb.png")
        thumb.assert_not_called()

    def test_web_thumb_renders_and_converts_in_virtual_display(self):
        d = self.t / "bg"
        d.mkdir()
        proc = mock.Mock()
        proc.wait.return_value = 0
        def start(cmd, **kwargs):
            self.assertIn("--silent", cmd)
            self.assertEqual(cmd[1:4], ["-a", "-s", "-screen 0 960x540x24"])  # xvfb-run defaults to 640x480
            self.assertTrue(kwargs["start_new_session"])
            self.assertEqual(kwargs["env"]["GDK_BACKEND"], "x11")
            self.assertNotIn("WAYLAND_DISPLAY", kwargs["env"])
            Path(cmd[cmd.index("--screenshot") + 1]).write_bytes(b"png")
            return proc
        with mock.patch("wallforge.webplayer.available", return_value=""), \
             mock.patch("wallforge.importer.shutil.which", return_value="/usr/bin/xvfb-run"), \
             mock.patch("wallforge.importer.subprocess.Popen", side_effect=start), \
             mock.patch.object(importer, "_thumb", return_value="preview.jpg") as convert:
            self.assertEqual(importer._web_thumb(d), "preview.jpg")
        self.assertEqual(convert.call_args.args[1], d / "preview.jpg")
        self.assertEqual(convert.call_args.kwargs, {"video": False})

    def test_web_thumb_does_not_need_layer_shell(self):
        # screenshot mode opens a normal window, so GNOME (no layer-shell) still gets previews
        d = self.t / "bg"
        d.mkdir()
        proc = mock.Mock()
        proc.wait.return_value = 0
        def start(cmd, **kwargs):
            Path(cmd[cmd.index("--screenshot") + 1]).write_bytes(b"png")
            return proc
        with mock.patch("wallforge.webplayer.available", return_value="gtk4-layer-shell"), \
             mock.patch("wallforge.importer.shutil.which", return_value="/usr/bin/xvfb-run"), \
             mock.patch("wallforge.importer.subprocess.Popen", side_effect=start), \
             mock.patch.object(importer, "_thumb", return_value="preview.jpg"):
            self.assertEqual(importer._web_thumb(d), "preview.jpg")

    def test_web_thumb_unavailable_and_timeout(self):
        d = self.t / "bg"
        d.mkdir()
        with mock.patch("wallforge.webplayer.available", return_value="webkitgtk-6.0, gtk4-layer-shell"):
            self.assertIsNone(importer._web_thumb(d))
        proc = mock.Mock(pid=123)
        proc.wait.side_effect = [importer.subprocess.TimeoutExpired("x", 15), 0]
        with mock.patch("wallforge.webplayer.available", return_value=""), \
             mock.patch("wallforge.importer.shutil.which", return_value="/usr/bin/xvfb-run"), \
             mock.patch("wallforge.importer.subprocess.Popen", return_value=proc), \
             mock.patch("wallforge.importer.os.killpg") as kill:
            self.assertIsNone(importer._web_thumb(d))
            kill.assert_called_once_with(123, importer.signal.SIGKILL)

    def test_video_is_symlinked_and_removal_keeps_original(self):
        v = self.t / "movie.mp4"
        v.write_bytes(b"data")
        d = importer.import_path(v)
        self.assertTrue((d / "video.mp4").is_symlink())
        it = library.scan()[0]
        self.assertEqual((it.kind, it.source), ("video", "Local"))
        importer.remove(d)
        self.assertTrue(v.exists())
        self.assertEqual(library.scan(), [])

    def test_html_folder(self):
        w = self.t / "page"
        w.mkdir()
        (w / "index.html").write_text("<html>")
        self.assertEqual(library.scan() or None, None)
        importer.import_path(w)
        self.assertEqual(library.scan()[0].kind, "web")

    def test_remove_refuses_outside_library(self):
        other = self.t / "workshop" / "123"
        other.mkdir(parents=True)
        with self.assertRaises(importer.ImportError_):
            importer.remove(other)
        self.assertTrue(other.exists())

    def test_unsupported_extension(self):
        f = self.t / "x.txt"
        f.write_text("x")
        with self.assertRaises(importer.ImportError_):
            importer.import_path(f)


if __name__ == "__main__":
    unittest.main()
