"""Tests for settings storage, defaults and migrations (no GTK needed)."""
import json
import subprocess
import sys
import tempfile
import threading
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
        self.assertEqual(cfg.data["version"], 6)
        self.assertFalse(self.file.exists(), "reading defaults must not write a file")

    def test_v4_file_is_migrated(self):
        self.write({"version": 4, "ultrawide_on_16_9": True,
                    "engine": {"fps": 60}, "outputs": {"DP-2": {"id": "42"}}, "favorites": ["42"]})
        cfg = config.Config()
        self.assertEqual(cfg.data["version"], 6)
        self.assertEqual(cfg.data["engine"]["fps"], 60, "existing settings are kept")
        self.assertEqual(cfg.data["engine"]["render_scale"], 100, "new keys come from the defaults")
        self.assertEqual(cfg.data["outputs"], {"DP-2": {"id": "42"}})
        self.assertEqual(cfg.data["favorites"], ["42"])
        self.assertEqual(json.loads(self.file.read_text())["version"], 6, "the migration is saved")

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

    def test_explicit_output_removal_writes_both_fields_and_is_not_resurrected(self):
        cfg = config.Config()
        cfg.data["outputs"] = {"DP-1": {"id": "1"}}
        cfg.data["displaced_outputs"] = [{"connector": "DP-2", "entry": {"id": "2", "ident": "B"}}]
        cfg.save()
        stale = config.Config()
        newer = config.Config()
        newer.data["displaced_outputs"] = [{"connector": "DP-3", "entry": {"id": "3", "ident": "C"}}]
        newer.save()
        stale.data["outputs"].clear()
        stale.save()  # an explicit change writes both of this instance's fields
        result = config.Config()
        self.assertEqual(result.data["outputs"], {})
        self.assertEqual(result.data["displaced_outputs"], stale.data["displaced_outputs"])
        newer.data["window"]["w"] = 900
        newer.save()  # its unchanged assignment snapshot must not restore the removal
        result = config.Config()
        self.assertEqual(result.data["outputs"], {})
        self.assertEqual(result.data["displaced_outputs"], stale.data["displaced_outputs"])

    def test_explicit_displaced_removal_writes_both_fields(self):
        cfg = config.Config()
        cfg.data["outputs"] = {"DP-1": {"id": "1"}}
        cfg.data["displaced_outputs"] = [{"connector": "DP-2", "entry": {"id": "2", "ident": "B"}}]
        cfg.save()
        stale = config.Config()
        cfg.data["outputs"] = {"DP-3": {"id": "3"}}
        cfg.save()
        stale.data["displaced_outputs"].clear()
        stale.save()
        result = config.Config()
        self.assertEqual(result.data["outputs"], {"DP-1": {"id": "1"}})
        self.assertEqual(result.data["displaced_outputs"], [])

    def test_concurrent_output_updates_do_not_lose_each_other(self):
        from wallforge import daemon

        cfg1, cfg2 = config.Config(), config.Config()
        cfg1.save()
        first_read = threading.Event()
        second_written = threading.Event()
        real_resolve, real_write = daemon.resolve_state, config.Config._write
        errors = []

        def resolve(*args):
            result = real_resolve(*args)
            if threading.current_thread().name == "first":
                first_read.set()
                second_written.wait(1)
            return result

        def write(cfg):
            real_write(cfg)
            if threading.current_thread().name == "second":
                second_written.set()

        def update(cfg, name):
            try:
                cfg.update_outputs({name: {"id": name}}, [])
            except Exception as e:
                errors.append(e)

        with mock.patch.object(daemon, "resolve_state", side_effect=resolve), mock.patch.object(config.Config, "_write", write):
            first = threading.Thread(target=update, args=(cfg1, "DP-1"), name="first")
            second = threading.Thread(target=update, args=(cfg2, "DP-2"), name="second")
            first.start()
            self.assertTrue(first_read.wait(3))
            second.start()
            first.join(5)
            second.join(5)
        self.assertFalse(first.is_alive() or second.is_alive(), "updates deadlocked")
        self.assertEqual(errors, [])
        self.assertEqual(config.Config().data["outputs"], {"DP-1": {"id": "DP-1"}, "DP-2": {"id": "DP-2"}})

    def test_output_lock_is_reentrant_and_excludes_other_processes(self):
        probe = ("import fcntl, sys\n"
                 "with open(sys.argv[1], 'a+') as f:\n"
                 " try: fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)\n"
                 " except BlockingIOError: sys.exit(1)\n")
        lockfile = str(self.dir / "config.lock")
        with config.output_lock(), config.output_lock():
            result = subprocess.run([sys.executable, "-c", probe, lockfile], timeout=5, check=False)
            self.assertEqual(result.returncode, 1)
        result = subprocess.run([sys.executable, "-c", probe, lockfile], timeout=5, check=False)
        self.assertEqual(result.returncode, 0)

    def test_update_outputs_can_migrate_while_locked(self):
        self.write({"version": 4, "outputs": {"DP-1": {"id": "1"}}})
        cfg = config.Config()
        self.write({"version": 4, "outputs": {"DP-1": {"id": "1"}}})
        cfg.update_outputs({"DP-2": {"id": "2"}}, [])
        self.assertEqual(config.Config().data["outputs"], {"DP-1": {"id": "1"}, "DP-2": {"id": "2"}})


if __name__ == "__main__":
    unittest.main()
