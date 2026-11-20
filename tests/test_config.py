"""Tests for settings storage, defaults and migrations (no GTK needed)."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from wallforge import config


class ConfigTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.file = self.dir / "config.json"
        self.patches = [mock.patch.object(config, "CONFIG_DIR", self.dir),
                        mock.patch.object(config, "CONFIG_FILE", self.file)]
        for p in self.patches:
            p.start()
        self.addCleanup(lambda: [p.stop() for p in self.patches])

    def write(self, data):
        self.file.write_text(json.dumps(data), encoding="utf-8")

    def test_defaults_are_complete(self):
        cfg = config.Config()
        e = cfg.data["engine"]
        for key in ("fps", "render_scale", "reduce_on_battery", "pause_on_battery", "scaling", "layer"):
            self.assertIn(key, e)
        for key in ("rotate_minutes", "rotate_shuffle", "rotate_favorites", "last_rotate"):
            self.assertIn(key, cfg.data)
        self.assertEqual(cfg.data["version"], 5)
        self.assertFalse(self.file.exists(), "reading defaults must not write a file")

    def test_v4_file_is_migrated(self):
        self.write({"version": 4, "ultrawide_on_16_9": True,
                    "engine": {"fps": 60}, "outputs": {"DP-2": {"id": "42"}}, "favorites": ["42"]})
        cfg = config.Config()
        self.assertEqual(cfg.data["version"], 5)
        self.assertEqual(cfg.data["engine"]["fps"], 60, "existing settings are kept")
        self.assertEqual(cfg.data["engine"]["render_scale"], 100, "new keys come from the defaults")
        self.assertEqual(cfg.data["outputs"], {"DP-2": {"id": "42"}})
        self.assertEqual(cfg.data["favorites"], ["42"])
        self.assertEqual(json.loads(self.file.read_text())["version"], 5, "the migration is saved")

    def test_broken_file_falls_back_to_defaults(self):
        self.file.write_text("{not json", encoding="utf-8")
        cfg = config.Config()
        self.assertEqual(cfg.data["engine"]["fps"], 30)

    def test_volume_is_global_times_wallpaper(self):
        cfg = config.Config()
        cfg.data["engine"]["volume"] = 50
        self.assertEqual(cfg.volume_for("x"), 50, "a fresh wallpaper plays at the global volume")
        cfg.data["volumes"]["x"] = 40
        self.assertEqual(cfg.volume_for("x"), 20)
        cfg.data["volumes"]["x"] = 0
        self.assertEqual(cfg.volume_for("x"), 0)
        cfg.data["engine"]["silent"] = True
        self.assertEqual(cfg.volume_for("x"), 0, "mute wins")

    def test_volume_survives_odd_values(self):
        cfg = config.Config()
        cfg.data["volumes"]["x"] = "loud"
        self.assertEqual(cfg.volume_for("x"), 100)

    def test_property_values_are_per_wallpaper(self):
        cfg = config.Config()
        cfg.set_value("42", "schemecolor", "1 0 0")
        self.assertEqual(cfg.values_for("42"), {"schemecolor": "1 0 0"})
        cfg.reset_values("42")
        self.assertEqual(cfg.values_for("42"), {})


if __name__ == "__main__":
    unittest.main()
