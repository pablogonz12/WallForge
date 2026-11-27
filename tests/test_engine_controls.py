"""Playback control arguments, without a display or renderer."""
import copy
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from wallforge import config, engine, library


class PlaybackControlsTest(unittest.TestCase):
    def setUp(self):
        self.cfg = SimpleNamespace(
            data=copy.deepcopy(config.DEFAULTS),
            values_for=lambda _id: {},
            volume_for=lambda _id: 100,
        )
        self.cfg.data["engine"].update(scaling="fill", reduce_on_battery=False)

    def item(self, kind):
        return library.Item(id="fixture", dir=Path("/nonexistent/wallforge-fixture"),
                            title="Fixture", type=kind, preview=None)

    def test_control_flags_by_renderer(self):
        with (mock.patch.object(engine.paths, "find_render", return_value=Path("/renderer")),
              mock.patch.object(engine.paths, "find_engine", return_value=Path("/legacy")),
              mock.patch.object(engine.paths, "find_assets", return_value=Path("/assets")),
              mock.patch.object(engine, "window_mode", return_value=False)):
            for mouse in (False, True):
                for parallax in (False, True):
                    with self.subTest(disable_mouse=mouse, disable_parallax=parallax):
                        self.cfg.data["engine"].update(disable_mouse=mouse, disable_parallax=parallax)
                        web = engine.build_web_args(self.cfg, self.item("web"), "test-output")
                        self.assertEqual("--mouse" in web, not mouse)
                        self.assertNotIn("--disable-mouse", web)
                        self.assertNotIn("--disable-parallax", web)
                        for kind in ("scene", "video"):
                            native = engine.build_native_args(self.cfg, self.item(kind), "test-output")
                            for flag in ("--mouse", "--disable-mouse", "--disable-parallax"):
                                self.assertNotIn(flag, native)
                        legacy = engine.build_args(self.cfg, self.item("scene"), "test-output")
                        self.assertEqual("--disable-mouse" in legacy, mouse)
                        self.assertEqual("--disable-parallax" in legacy, parallax)
                        self.assertNotIn("--mouse", legacy)


if __name__ == "__main__":
    unittest.main()
