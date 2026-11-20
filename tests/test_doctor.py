import unittest

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
