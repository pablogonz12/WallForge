"""Tests for the session daemon: identity matching, single-instance lock, supervisor decisions (fake engine)."""
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from wallforge import config, daemon, monitors


def live(*pairs):
    return [{"name": n, "ident": i} for n, i in pairs]


class ResolveTests(unittest.TestCase):
    def test_connector_wins_when_identity_agrees(self):
        m, out = daemon.resolve({"DP-2": {"id": "1", "ident": "A|x|1"}}, live(("DP-2", "A|x|1")))
        self.assertEqual(m["DP-2"]["id"], "1")

    def test_legacy_entry_without_ident_matches_by_connector_and_learns_it(self):
        m, out = daemon.resolve({"DP-2": {"id": "1"}}, live(("DP-2", "A|x|1")))
        self.assertEqual(m["DP-2"]["id"], "1")
        self.assertEqual(out["DP-2"]["ident"], "A|x|1")

    def test_renumbered_connectors_follow_identity(self):
        saved = {"DP-3": {"id": "1", "ident": "A|x|1"}, "DP-5": {"id": "2", "ident": "B|y|2"}}
        m, out = daemon.resolve(saved, live(("DP-3", "B|y|2"), ("DP-5", "A|x|1")))
        self.assertEqual(m["DP-3"]["id"], "2")
        self.assertEqual(m["DP-5"]["id"], "1")
        self.assertEqual(out["DP-3"]["id"], "2")  # config is re-keyed to the live names

    def test_moved_monitor_is_found_by_identity(self):
        m, out = daemon.resolve({"DP-3": {"id": "1", "ident": "A|x|1"}}, live(("DP-7", "A|x|1")))
        self.assertEqual(m["DP-7"]["id"], "1")
        self.assertNotIn("DP-3", out)

    def test_different_monitor_on_same_connector_is_not_matched(self):
        m, _ = daemon.resolve({"DP-3": {"id": "1", "ident": "A|x|1"}}, live(("DP-3", "C|z|9")))
        self.assertEqual(m, {})

    def test_unplugged_assignment_is_kept(self):
        _, out = daemon.resolve({"DP-3": {"id": "1", "ident": "A|x|1"}}, live(("DP-2", "B|y|2")))
        self.assertIn("DP-3", out)

    def test_identical_twins_use_connectors_and_each_entry_once(self):
        saved = {"DP-1": {"id": "1", "ident": "A|x|"}, "DP-2": {"id": "2", "ident": "A|x|"}}
        m, _ = daemon.resolve(saved, live(("DP-1", "A|x|"), ("DP-2", "A|x|")))
        self.assertEqual((m["DP-1"]["id"], m["DP-2"]["id"]), ("1", "2"))

    def test_make_ident_drops_unknowns(self):
        self.assertEqual(monitors.make_ident("Dell", "U2720Q", "ABC"), "Dell|U2720Q|ABC")
        self.assertEqual(monitors.make_ident("Unknown", "Unknown", "0"), "")
        self.assertEqual(monitors.make_ident("Dell", "U2720Q", "Unknown"), "Dell|U2720Q|")


class LockTests(unittest.TestCase):
    def test_second_instance_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with mock.patch.object(daemon, "LOCK", Path(d) / "daemon.lock"), mock.patch.object(daemon.engine, "STATE", Path(d)):
                self.assertIsNone(daemon.running_pid())
                self.assertTrue(daemon.acquire_lock())
                self.assertEqual(daemon.running_pid(), os.getpid())
                daemon._lock_fh.close()  # releases the flock
                daemon._lock_fh = None
                self.assertIsNone(daemon.running_pid())


class GuardTests(unittest.TestCase):
    def test_covered_by_later_background_surface(self):
        L = [("wallforge", "DP-2", "Background"), ("noctalia-wallpaper", "DP-2", "Background")]
        self.assertEqual(daemon.covered(L, "DP-2"), "noctalia-wallpaper")

    def test_on_top_or_other_output_or_layer_is_fine(self):
        L = [("noctalia-wallpaper", "DP-2", "Background"), ("wallforge", "DP-2", "Background"),
             ("bar", "DP-2", "Top"), ("swaybg", "HDMI-A-1", "Background")]
        self.assertEqual(daemon.covered(L, "DP-2"), "")
        self.assertEqual(daemon.covered(L, "HDMI-A-1"), "")  # no wallforge there at all


class FakeEngine:
    def __init__(self):
        self.alive, self.started, self.stopped, self.flag, self.layer = set(), [], [], set(), "background"
        self.flag_busy = set()

    def pause_for_battery(self, cfg): return False
    def is_stopped(self, o): return o in self.flag
    def busy(self, o): return o in self.flag_busy
    def alive_fast(self, o): return o in self.alive
    def current_layer(self, o): return self.layer
    def stop(self, o): self.stopped.append(o); self.alive.discard(o)

    def start(self, cfg, item, o):
        self.started.append(o)
        self.alive.add(o)


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.t = [1000.0]
        self.eng = FakeEngine()
        self.sup = daemon.Supervisor(self.eng, clock=lambda: self.t[0])
        self.sup.items = {"1": SimpleNamespace(id="1", broken="")}
        self.sup._scanned = 1e12
        self.cfg = SimpleNamespace(data={"outputs": {"DP-2": {"id": "1", "ident": "A"}, "HDMI-A-2": {"id": "1", "ident": "B"}}})
        self.live = live(("DP-2", "A"), ("HDMI-A-2", "B"))

    def tick(self, layers=None, dt=1.0):
        self.t[0] += dt
        self.sup.tick(self.cfg, self.live, layers)

    def test_starts_all_outputs_in_one_tick_and_adopts_running_ones(self):
        self.eng.alive.add("DP-2")
        self.tick()
        self.assertEqual(self.eng.started, ["HDMI-A-2"])
        self.eng.alive.clear()
        self.eng.started.clear()
        self.sup = daemon.Supervisor(self.eng, clock=lambda: self.t[0])
        self.sup.items, self.sup._scanned = {"1": SimpleNamespace(id="1", broken="")}, 1e12
        self.tick()
        self.assertEqual(sorted(self.eng.started), ["DP-2", "HDMI-A-2"])

    def test_crashed_renderer_comes_back_with_backoff_then_gives_up(self):
        self.tick()
        for _ in range(40):  # it dies every time we look, for 80 s
            self.eng.alive.discard("DP-2")
            self.tick(dt=2)
        n = self.eng.started.count("DP-2")
        self.assertGreaterEqual(n, 3)
        self.assertIn("DP-2", self.sup.gave_up)
        self.eng.alive.discard("DP-2")
        self.tick(dt=5)
        self.assertEqual(self.eng.started.count("DP-2"), n)  # still given up
        self.tick(dt=daemon.GIVE_UP_FOR)
        self.assertEqual(self.eng.started.count("DP-2"), n + 1)  # cool-down over: tries again

    def test_first_crash_is_restarted_immediately(self):
        self.tick()
        self.eng.alive.discard("DP-2")
        self.eng.started.clear()
        self.tick(dt=0.01)  # right after the crash, no backoff wait
        self.assertEqual(self.eng.started, ["DP-2"])
        self.eng.alive.discard("DP-2")
        self.eng.started.clear()
        self.tick(dt=0.01)  # the second crash in a row waits
        self.assertEqual(self.eng.started, [])
        self.assertGreater(self.sup.wait_hint(self.cfg, self.live, 5.0), 0.05 - 1e-9)
        self.tick(dt=1)
        self.assertEqual(self.eng.started, ["DP-2"])

    def test_gui_switch_or_stop_in_progress_is_not_a_crash(self):
        self.eng.alive.add("HDMI-A-2")
        self.eng.flag_busy.add("DP-2")  # engine.stop/start running on it
        self.tick()
        self.assertEqual(self.eng.started, [])
        self.assertEqual(self.sup.starts.get("DP-2"), None)

    def test_user_stopped_output_is_not_resurrected(self):
        self.eng.flag.add("DP-2")
        self.tick()
        self.assertEqual(self.eng.started, ["HDMI-A-2"])

    def test_unplugged_output_gets_its_renderer_stopped(self):
        self.eng.alive.update({"DP-2", "HDMI-A-2"})
        self.live = live(("DP-2", "A"))
        self.tick()
        self.assertEqual(self.eng.stopped, ["HDMI-A-2"])

    def test_renumbered_connector_still_starts_the_right_wallpaper(self):
        self.cfg.data["outputs"] = {"DP-3": {"id": "1", "ident": "A"}}
        self.live = live(("DP-5", "A"))
        self.tick()
        self.assertEqual(self.eng.started, ["DP-5"])

    def test_stacking_guard_recreates_once_then_rate_limits(self):
        self.eng.alive.update({"DP-2", "HDMI-A-2"})
        bad = [("wallforge", "DP-2", "Background"), ("noctalia-wallpaper", "DP-2", "Background")]
        self.tick(bad, dt=5)
        self.assertEqual(self.eng.started, ["DP-2"])
        self.tick(bad, dt=1)
        self.assertEqual(self.eng.started, ["DP-2"])  # too soon
        self.tick(bad, dt=20)
        self.assertEqual(self.eng.started, ["DP-2", "DP-2"])

    def test_guard_skips_bottom_layer(self):
        self.eng.alive.update({"DP-2", "HDMI-A-2"})
        self.eng.layer = "bottom"
        self.tick([("wallforge", "DP-2", "Background"), ("x", "DP-2", "Background")], dt=5)
        self.assertEqual(self.eng.started, [])


class MigrationTests(unittest.TestCase):
    def test_v6_turns_autostart_on_for_users_with_wallpapers(self):
        import json
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "config.json"
            f.write_text(json.dumps({"version": 5, "autostart": False, "outputs": {"DP-1": {"id": "1"}}}))
            with mock.patch.object(config, "CONFIG_DIR", Path(d)), mock.patch.object(config, "CONFIG_FILE", f):
                self.assertTrue(config.Config().data["autostart"])
                f.write_text(json.dumps({"version": 5, "autostart": False, "outputs": {}}))
                self.assertFalse(config.Config().data["autostart"])



class XdgMigrationTests(unittest.TestCase):
    def test_old_restore_entry_is_rewritten_in_place(self):
        from wallforge import autostart
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "dev.wallforge.Wallforge-restore.desktop"
            f.write_text(f"[Desktop Entry]\nExec={autostart.launcher()} --restore\n")
            with mock.patch.object(autostart, "XDG_FILE", f):
                self.assertTrue(autostart._write(f, autostart.xdg_text()))
                self.assertIn("--daemon", f.read_text())
                self.assertNotIn("--restore", f.read_text())
                self.assertFalse(autostart._write(f, autostart.xdg_text()))  # idempotent


if __name__ == "__main__":
    unittest.main()
