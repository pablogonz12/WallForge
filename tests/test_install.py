"""Exercise install.sh in a synthetic checkout with no desktop or session tools."""

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

INSTALL = Path(__file__).resolve().parents[1] / "install.sh"


class InstallTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.source = root / "checkout"
        self.source.mkdir()
        shutil.copyfile(INSTALL, self.source / "install.sh")
        for name in ("wallforge.sh", "wallforge/__init__.py", "data/dev.wallforge.Wallforge.desktop.in",
                     "assets/wallforge.svg", "assets/wallforge-symbolic.svg"):
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("@EXEC@\n", encoding="utf-8")

        self.tools = root / "tools"
        self.tools.mkdir()
        for tool in ("chmod", "cp", "dirname", "ln", "mkdir", "readlink", "sed"):
            target = shutil.which(tool)
            self.assertIsNotNone(target)
            (self.tools / tool).symlink_to(target)
        self.prefix = root / "missing-parent" / "install"
        self.bin = root / "bin"
        self.env = {
            "HOME": str(root / "home"),
            "PATH": str(self.tools),
            "XDG_BIN_HOME": str(self.bin),
            "XDG_DATA_HOME": str(root / "data"),
        }

    def install(self, prefix):
        return subprocess.run(
            ["/bin/bash", str(self.source / "install.sh"), "--no-build"],
            cwd=self.source, env={**self.env, "WALLFORGE_PREFIX": str(prefix)},
            text=True, capture_output=True, timeout=10,
        )

    def test_missing_rsync_fails_before_creating_cross_prefix_install(self):
        result = self.install(self.prefix)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("install rsync", result.stderr.lower())
        self.assertFalse(self.prefix.parent.exists())
        self.assertFalse(self.bin.exists())

    def test_in_place_install_via_alias_does_not_need_rsync(self):
        alias = self.source.with_name("alias")
        alias.symlink_to(self.source, target_is_directory=True)
        result = self.install(alias)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.bin / "wallforge").resolve(), self.source / "wallforge.sh")
