"""Commons provider parsing (saved API response, no network) and Uninstall logic."""
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from wallforge import commons, importer, library, paths, uninstall

FIX = Path(__file__).parent / "fixtures" / "commons_search.json"


class Parse(unittest.TestCase):
    def setUp(self):
        self.data = json.loads(FIX.read_text())
        self.items = commons.parse(self.data)

    def test_filters(self):
        titles = [i["title"] for i in self.items]
        self.assertNotIn("Bopath Ella", titles)  # 320x240
        self.assertFalse(any("Vettisfossen" in t for t in titles))  # portrait
        self.assertFalse(any("Kawaida" in t for t in titles))  # portrait, low res
        self.assertEqual(len(self.items), 3)

    def test_fields(self):
        r = next(i for i in self.items if "Karkloof" in i["title"])
        self.assertEqual(r["license"], "CC BY 2.0")
        self.assertTrue(r["author"] and "<" not in r["author"])
        self.assertTrue(r["preview"].startswith("https://"))
        self.assertTrue(r["url"].startswith("https://commons.wikimedia.org/wiki/File:"))
        self.assertIn("CC BY 2.0", commons.attribution(r))

    def test_best_source_prefers_1080_webm(self):
        r = next(i for i in self.items if "Laussa" in i["title"])
        self.assertEqual(r["download"]["height"], 1080)
        self.assertTrue(r["download"]["url"].endswith(".1080p.vp9.webm"))
        self.assertEqual(r["download"]["ext"], ".webm")

    def test_licences(self):
        for ok in ("CC BY-SA 4.0", "CC BY 3.0", "Public domain", "CC0", "CC BY 2.0"):
            self.assertTrue(commons.licence_ok(ok), ok)
        for bad in ("CC BY-NC 4.0", "CC BY-ND 2.0", "Copyrighted free use", "", "GFDL", "Fair use"):
            self.assertFalse(commons.licence_ok(bad), bad)

    def test_nonfree_dropped(self):
        d = json.loads(FIX.read_text())
        d["query"]["pages"][3]["videoinfo"][0]["extmetadata"]["LicenseShortName"]["value"] = "CC BY-NC 4.0"
        self.assertEqual(len(commons.parse(d)), 2)

    def test_empty(self):
        self.assertEqual(commons.parse({"batchcomplete": True}), [])


class Query(unittest.TestCase):
    def search(self, *args):
        with mock.patch.object(commons, "_get", return_value={"batchcomplete": True}) as get:
            commons.browse(*args)
        return get.call_args.args[0]

    def test_topic_searches_its_subcategories(self):
        q = self.search("", "Mountains")
        self.assertEqual(q["gsrsearch"], 'filetype:video deepcat:"Videos of mountains"')

    def test_query_words_follow_the_category(self):
        self.assertEqual(self.search("sunrise", "Waterfalls")["gsrsearch"],
                         'filetype:video deepcat:"Videos of waterfalls" sunrise')

    def test_new_topics(self):
        for label, cat in [("Oceans", "Videos of oceans"), ("Snow", "Videos of snow"), ("Fire", "Videos of fire"),
                           ("Earth from space", "Videos of Earth from space")]:
            self.assertEqual(commons.TOPICS[label], cat)
            self.assertIn(f'deepcat:"{cat}"', self.search("", label)["gsrsearch"])

    def test_only_valid_videoinfo_props(self):
        # Commons warns about "duration" and "thumbnail": size already returns the duration, viurlwidth the thumbnail.
        self.assertEqual(self.search("", "Nature")["viprop"].split("|"),
                         ["url", "size", "mime", "derivatives", "extmetadata", "user"])

    def test_cli_rejects_unknown_topic(self):
        with mock.patch.object(commons, "browse") as browse, mock.patch("sys.stderr") as err:
            self.assertEqual(commons.main(["--topic", "Nope"]), 2)
        browse.assert_not_called()
        self.assertIn("Waterfalls", "".join(c.args[0] for c in err.write.call_args_list))


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)
        self.lib = self.t / "lib"
        p = mock.patch.dict(os.environ, {"WALLFORGE_LIBRARY": str(self.lib)})
        p.start()
        self.addCleanup(p.stop)
        self.addCleanup(self.tmp.cleanup)
        self.ws = self.t / "workshop"
        self.ws.mkdir()
        for name, rv in (("workshop_dirs", [self.ws]), ("we_dirs", [])):
            q = mock.patch.object(paths, name, return_value=rv)
            q.start()
            self.addCleanup(q.stop)


class Import(Base):
    def test_download_import_records_licence(self):
        r = next(i for i in commons.parse(json.loads(FIX.read_text())) if "Karkloof" in i["title"])
        body = b"\x1aE\xdf\xa3fake-webm"

        class Resp:
            headers = {"Content-Length": str(len(body))}
            def __init__(s): s.b = body
            def read(s, n=-1):
                b, s.b = s.b, b""
                return b
            def __enter__(s): return s
            def __exit__(s, *a): pass
        with mock.patch.object(commons.urllib.request, "urlopen", return_value=Resp()):
            d = commons.download(r)
            again = commons.download(r)  # already installed: no second copy
        self.assertEqual(d, again)
        items = library.scan()
        self.assertEqual(len(items), 1)
        it = items[0]
        self.assertEqual((it.source, it.source_label), ("Wikimedia", "Wikimedia"))
        self.assertEqual(it.raw["license"], "CC BY 2.0")
        self.assertTrue(it.raw["author"])
        self.assertIn("Wikimedia Commons", it.credit)
        self.assertEqual(it.type, "video")
        self.assertTrue(commons.installed(r["pageid"]))
        self.assertEqual(uninstall.plan(it).action, "delete")
        self.assertIn("downloaded copy", uninstall.plan(it).message)
        uninstall.run(it)
        self.assertFalse(d.exists())
        self.assertIsNone(commons.installed(r["pageid"]))


class Uninstall(Base):
    def test_local_symlink_keeps_original(self):
        v = self.t / "clip.mp4"
        v.write_bytes(b"x")
        d = importer.import_video(v)
        it = library.scan()[0]
        self.assertIn("original file is kept", uninstall.plan(it).message)
        self.assertEqual(uninstall.run(it), "deleted")
        self.assertFalse(d.exists())
        self.assertTrue(v.exists())

    def test_workshop_opens_steam_and_keeps_folder(self):
        d = self.ws / "123456"
        d.mkdir()
        (d / "project.json").write_text(json.dumps({"type": "video", "file": "a.mp4", "title": "W"}))
        with mock.patch.object(paths, "APPID", self.ws.name):  # workshop_id wants the app-id dir name
            it = library.scan()[0]
            self.assertEqual(it.source, "Workshop")
            self.assertEqual(uninstall.plan(it).action, "unsubscribe")
            opened = []
            self.assertEqual(uninstall.run(it, opened.append), "unsubscribe")
        self.assertEqual(opened, ["steam://url/CommunityFilePage/123456"])
        self.assertTrue(d.exists())

    def test_foreign_item_refused(self):
        it = library.Item(id="x", dir=self.t / "elsewhere", title="x", type="video", preview=None)
        (self.t / "elsewhere").mkdir()
        self.assertEqual(uninstall.plan(it).action, "none")
        with self.assertRaises(importer.ImportError_):
            uninstall.run(it)
        self.assertTrue((self.t / "elsewhere").exists())


if __name__ == "__main__":
    unittest.main()
