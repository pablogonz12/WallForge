"""Tests for the pause/resume decisions (fake engine, recorded signals)."""
import signal
import unittest

from wallforge import pauser


class Eng:
    def __init__(self):
        self.pids, self.native = {"DP-2": 100, "HDMI-A-2": 200}, {"DP-2", "HDMI-A-2"}

    def visible_pid(self, o): return self.pids.get(o)
    def pausable(self, o): return o in self.native


class PauserTests(unittest.TestCase):
    def setUp(self):
        self.sent = []
        self.eng = Eng()
        self.p = pauser.Pauser(self.eng, send=lambda pid, sig: self.sent.append((pid, sig)), covered_fn=lambda: set())
        self.outs = ["DP-2", "HDMI-A-2"]

    def test_covered_output_is_paused_once_and_others_untouched(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.p.apply(self.outs)
        self.assertEqual(self.sent, [(100, signal.SIGUSR2)])

    def test_uncovered_resumes(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.p.covered = set()
        self.p.apply(self.outs)
        self.assertEqual(self.sent, [(100, signal.SIGUSR2), (100, signal.SIGUSR1)])

    def test_overview_counts_as_uncovered(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.p.overview = True
        self.p.apply(self.outs)
        self.assertEqual(self.sent[-1], (100, signal.SIGUSR1))
        self.p.overview = False
        self.p.apply(self.outs)
        self.assertEqual(self.sent[-1], (100, signal.SIGUSR2))

    def test_new_renderer_on_a_covered_output_is_paused_and_old_pid_is_not_resumed(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.eng.pids["DP-2"] = 101  # gapless switch / crash restart
        self.p.apply(self.outs)
        self.assertEqual(self.sent, [(100, signal.SIGUSR2), (101, signal.SIGUSR2)])

    def test_non_native_or_unready_renderers_are_never_signalled(self):
        self.eng.native.discard("DP-2")
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.assertEqual(self.sent, [])

    def test_setting_off_resumes(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        self.p.enabled = False
        self.p.apply(self.outs)
        self.assertEqual(self.sent[-1], (100, signal.SIGUSR1))

    def test_dead_pid_is_forgotten(self):
        self.p.covered = {"DP-2"}
        self.p.apply(self.outs)
        del self.eng.pids["DP-2"]
        self.p.apply(self.outs)
        self.assertNotIn("DP-2", self.p.paused)


if __name__ == "__main__":
    unittest.main()
