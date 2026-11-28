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

    def lively(self, name, extra, info=None):
        info = info or {"Type": "web", "FileName": "index.html", "Title": "T"}
        return self.zip(name, {"LivelyInfo.json": json.dumps(info), "index.html": "<html>", **extra})

    def assert_library_empty(self):
        self.assertEqual(list(self.lib.iterdir()) if self.lib.exists() else [], [])

    def test_traversal_zip_is_refused_before_anything_is_written(self):
        z = self.lively("slip.zip", {"pkg/../../../evil.txt": "x"})
        with self.assertRaises(importer.ImportError_) as c:
            importer.import_path(z)
        self.assertIn("Unsafe path", str(c.exception))
        self.assertFalse((self.t / "evil.txt").exists())
        self.assert_library_empty()

    def test_symlink_entry_is_refused(self):
        z = self.lively("link.zip", {})
        with zipfile.ZipFile(z, "a") as zf:
            zi = zipfile.ZipInfo("link")
            zi.external_attr = 0o120777 << 16
            zf.writestr(zi, "/etc/passwd")
        with self.assertRaises(importer.ImportError_) as c:
            importer.import_path(z)
        self.assertIn("Symlink", str(c.exception))
        self.assert_library_empty()

    def test_zip_over_unpacked_cap_is_refused_and_cleaned_up(self):
        z = self.lively("bomb.zip", {"big.bin": "x" * 1000})
        with mock.patch.object(importer, "MAX_UNPACKED", 500), self.assertRaises(importer.ImportError_) as c:
            importer.import_path(z)
        self.assertIn("more than", str(c.exception))
        self.assert_library_empty()

    def test_too_many_files_is_refused(self):
        z = self.lively("many.zip", {f"f{i}.txt": "" for i in range(5)})
        with mock.patch.object(importer, "MAX_FILES", 3), self.assertRaises(importer.ImportError_):
            importer.import_path(z)
        self.assert_library_empty()

    def test_filename_escaping_the_package_is_refused(self):
        z = self.lively("esc.zip", {}, {"Type": "web", "FileName": "../outside.html", "Title": "T"})
        with self.assertRaises(importer.ImportError_):
            importer.import_path(z)
        self.assert_library_empty()

    def test_missing_wallpaper_file_is_refused_and_cleaned_up(self):
        z = self.zip("gone.zip", {"LivelyInfo.json": json.dumps({"Type": "video", "FileName": "clip.mp4", "Title": "T"})})
        with self.assertRaises(importer.ImportError_) as c:
            importer.import_path(z)
        self.assertIn("clip.mp4", str(c.exception))
        self.assert_library_empty()


class LivelyPreviewTest(Base):
    def project(self, d):
        return json.loads((d / "project.json").read_text())

    def test_web_package_without_thumbnail_imports_without_preview(self):
        z = self.zip("w.zip", {"LivelyInfo.json": json.dumps({"Type": "web", "FileName": "index.html", "Title": "W"}),
                               "index.html": "<html>"})
        with mock.patch.object(importer, "_thumb") as thumb:
            d = importer.import_path(z)
        thumb.assert_not_called()  # web pages get no ffmpeg thumbnail
        self.assertNotIn("preview", self.project(d))
        self.assertFalse(library.scan()[0].broken)

    def test_video_without_thumbnail_gets_a_generated_preview(self):
        z = self.zip("v.zip", {"LivelyInfo.json": json.dumps({"Type": "video", "FileName": "clip.mp4", "Title": "V"}),
                               "clip.mp4": b"x"})
        with mock.patch.object(importer, "_thumb", return_value="preview.jpg") as thumb:
            d = importer.import_path(z)
        self.assertEqual(thumb.call_args.args, (d / "clip.mp4", d / "preview.jpg"))
        self.assertEqual(self.project(d)["preview"], "preview.jpg")

    def test_video_imports_when_thumbnail_generation_fails(self):
        z = self.zip("v.zip", {"LivelyInfo.json": json.dumps({"Type": "video", "FileName": "clip.mp4", "Title": "V"}),
                               "clip.mp4": b"x"})
        with mock.patch.object(importer, "_thumb", return_value=None):
            d = importer.import_path(z)
        self.assertNotIn("preview", self.project(d))
        self.assertFalse(library.scan()[0].broken)

    def test_declared_thumbnail_that_is_missing_or_outside_is_dropped(self):
        for thumb in ("thumb.jpg", "../thumb.jpg", "/etc/hostname"):
            with self.subTest(thumbnail=thumb):
                info = {"Type": "web", "FileName": "index.html", "Title": "W", "Thumbnail": thumb}
                z = self.zip("t.zip", {"LivelyInfo.json": json.dumps(info), "index.html": "<html>"})
                d = importer.import_path(z)
                self.assertNotIn("preview", self.project(d))

    def test_shipped_thumbnail_is_kept(self):
        info = {"Type": "web", "FileName": "index.html", "Title": "W", "Thumbnail": "thumb.jpg"}
        z = self.zip("t.zip", {"LivelyInfo.json": json.dumps(info), "index.html": "<html>", "thumb.jpg": b"jpg"})
        with mock.patch.object(importer, "_thumb") as thumb:
            d = importer.import_path(z)
        thumb.assert_not_called()
        self.assertEqual(self.project(d)["preview"], "thumb.jpg")


class LocalImportTest(Base):
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
