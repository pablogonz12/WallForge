import os
import tempfile
import unittest
from pathlib import Path

from wallforge import doctor


class ReportTests(unittest.TestCase):
    def test_all_ok_says_no_problems(self):
        text, code = doctor.report([(doctor.OK, "Python", "3.14"), (doctor.OK, "session", "niri on wayland")])
        self.assertEqual(code, 0)
        self.assertIn("No problems found.", text)

    def test_warnings_do_not_fail_the_exit_code(self):
        text, code = doctor.report([(doctor.OK, "session", "niri"), (doctor.WARN, "WebKit", "missing")])
        self.assertEqual(code, 0)
        self.assertIn("1 warning", text)

    def test_a_failure_fails_the_exit_code_and_names_the_count(self):
        text, code = doctor.report([(doctor.BAD, "session", "X11"), (doctor.WARN, "WebKit", "missing")])
        self.assertEqual(code, 1)
        self.assertIn("1 problem, 1 warning", text)
        self.assertIn("FAIL", text)

    def test_detail_is_aligned_behind_the_longest_label(self):
        text, _ = doctor.report([(doctor.OK, "a", "x"), (doctor.OK, "longer label", "y")])
        rows = [line for line in text.splitlines() if " x" in line or " y" in line]
        self.assertEqual(len({line.index("x") if line.endswith("x") else line.index("y") for line in rows}), 1)


class LastRunTests(unittest.TestCase):
    def _with_log(self, text: str):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "wallforge").mkdir()
            (Path(d) / "wallforge" / "DP-1.log").write_text(text)
            old = os.environ.get("XDG_RUNTIME_DIR")
            os.environ["XDG_RUNTIME_DIR"] = d
            try:
                return doctor.checks()
            finally:
                if old is None:
                    os.environ.pop("XDG_RUNTIME_DIR", None)
                else:
                    os.environ["XDG_RUNTIME_DIR"] = old

    def test_a_log_from_a_crashed_run_is_a_warning_not_a_crash(self):
        entries = self._with_log("starting\nwallforge-render: panic: boom\n")
        self.assertTrue(all(len(e) == 3 for e in entries), entries)
        self.assertIn("last run", [label for _, label, _ in entries])
        # the report still builds, and its exit code only reflects FAIL lines (a runner may have its own,
        # e.g. no layer-shell, so do not assume a healthy machine)
        text, code = doctor.report(entries)
        self.assertIn("last run", text)
        self.assertEqual(code, 1 if any(s == doctor.BAD for s, _, _ in entries) else 0)

    def test_an_empty_log_dir_adds_nothing(self):
        entries = self._with_log("")
        self.assertTrue(all(len(e) == 3 for e in entries), entries)


class ChecksTests(unittest.TestCase):
    def test_checks_are_well_formed(self):
        entries = doctor.checks()
        self.assertGreaterEqual(len(entries), 6)
        for status, label, detail in entries:
            self.assertIn(status, (doctor.OK, doctor.WARN, doctor.BAD))
            self.assertTrue(label)
            self.assertIsInstance(detail, str)

    def test_python_and_session_are_always_reported(self):
        labels = [label for _, label, _ in doctor.checks()]
        self.assertTrue(any(label.startswith("Python") for label in labels))
        self.assertIn("session", labels)


if __name__ == "__main__":
    unittest.main()
