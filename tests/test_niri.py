"""Tests for the niri overview-backdrop file management (no niri needed)."""
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from wallforge import niri


class NiriTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        (self.dir / "niri").mkdir()
        self.main = self.dir / "niri" / "config.kdl"
        self.main.write_text("input {}\n")
        env = {"XDG_CONFIG_HOME": str(self.dir), "NIRI_SOCKET": "/x"}
        p = mock.patch.dict(os.environ, env)
        p.start()
        self.addCleanup(p.stop)
        v = mock.patch.object(niri, "_validate", return_value="")
        v.start()
        self.addCleanup(v.stop)

    def test_include_once_and_toggle(self):
        self.assertEqual(niri.apply(True), "")
        mine = self.dir / "niri" / niri.FILE_NAME
        self.assertIn("place-within-backdrop true", mine.read_text())
        niri.apply(False)
        self.assertNotIn("place-within-backdrop", mine.read_text())
        niri.apply(True)
        self.assertEqual(self.main.read_text().count(niri.INCLUDE), 1)
        self.assertTrue((self.dir / "niri" / niri.BACKUP).exists())
        self.assertEqual((self.dir / "niri" / niri.BACKUP).read_text(), "input {}\n")

    def test_existing_include_untouched(self):
        txt = f'input {{}}\n{niri.INCLUDE}\n'
        self.main.write_text(txt)
        niri.apply(True)
        self.assertEqual(self.main.read_text(), txt)
        self.assertFalse((self.dir / "niri" / niri.BACKUP).exists())

    def test_invalid_restores(self):
        with mock.patch.object(niri, "_validate", return_value="bad"):
            self.assertEqual(niri.apply(True), "bad")
        self.assertEqual(self.main.read_text(), "input {}\n")
        self.assertFalse((self.dir / "niri" / niri.FILE_NAME).exists())

    def test_noop_off_niri(self):
        with mock.patch.dict(os.environ, {"NIRI_SOCKET": "", "XDG_CURRENT_DESKTOP": "GNOME"}):
            niri.apply(True)
        self.assertFalse((self.dir / "niri" / niri.FILE_NAME).exists())


if __name__ == "__main__":
    unittest.main()
