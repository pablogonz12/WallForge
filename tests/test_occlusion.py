"""Tests for wallforge.occlusion decision logic (no niri needed)."""
import unittest

from wallforge import occlusion

OUT = {"DP-2": {"logical": {"width": 2560, "height": 1440}}, "HDMI-A-2": {"logical": {"width": 3840, "height": 2160}}}
WS = [{"id": 5, "output": "DP-2", "is_active": True}, {"id": 6, "output": "DP-2", "is_active": False},
      {"id": 2, "output": "HDMI-A-2", "is_active": True}]


def win(ws, size, pos=(0.0, 0.0), floating=False):
    return {"workspace_id": ws, "is_floating": floating,
            "layout": {"window_size": list(size), "tile_pos_in_workspace_view": list(pos) if pos else None}}


class OcclusionTest(unittest.TestCase):
    def test_fullscreen_covers(self):
        self.assertEqual(occlusion.covered_from_niri(OUT, WS, [win(5, (2560, 1440))]), {"DP-2"})

    def test_gapped_tile_does_not(self):
        self.assertEqual(occlusion.covered_from_niri(OUT, WS, [win(5, (2538, 1418), (10, 10))]), set())

    def test_inactive_workspace_floating_and_offscreen(self):
        w = [win(6, (2560, 1440)), win(5, (2560, 1440), floating=True), win(5, (2560, 1440), pos=None)]
        self.assertEqual(occlusion.covered_from_niri(OUT, WS, w), set())

    def test_per_output(self):
        w = [win(2, (3840, 2160)), win(5, (800, 600))]
        self.assertEqual(occlusion.covered_from_niri(OUT, WS, w), {"HDMI-A-2"})

    def test_empty(self):
        self.assertEqual(occlusion.covered_from_niri(None, None, None), set())


if __name__ == "__main__":
    unittest.main()
