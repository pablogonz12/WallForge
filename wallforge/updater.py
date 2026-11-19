"""Check GitHub for a newer Wallforge and update when it is a git checkout.

Git checkout: `git pull --ff-only`. Package install (AUR etc.): only tell the user, the package manager updates it.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO = "https://github.com/pgm1207/wallforge"
ROOT = Path(__file__).resolve().parent.parent


def _git(*args, cwd=ROOT, timeout=25) -> str:
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if r.returncode:
        raise RuntimeError(r.stderr.strip() or "git failed")
    return r.stdout.strip()


def kind() -> str:
    """'git' for a clone, 'package' for a system install, 'other'."""
    if (ROOT / ".git").exists():
        return "git"
    return "package" if str(ROOT).startswith(("/usr", "/opt")) else "other"


def current_commit() -> str | None:
    if kind() == "git":
        try:
            return _git("rev-parse", "HEAD")
        except Exception:
            return None
    try:
        from . import _build  # written by the AUR package at build time
        return _build.COMMIT
    except Exception:
        return None


def latest_commit() -> str:
    out = _git("ls-remote", REPO, "HEAD", cwd="/")
    return out.split()[0]


def check() -> dict:
    """{'available': bool, 'kind': str, 'current': str|None, 'latest': str}. Raises on network trouble."""
    latest = latest_commit()
    cur = current_commit()
    return {"available": bool(cur) and cur != latest, "kind": kind(), "current": cur, "latest": latest}


def _healthy() -> bool:
    """The freshly pulled code must at least compile and import its non-GUI modules."""
    r = subprocess.run([sys.executable, "-I", "-m", "compileall", "-q", str(ROOT / "wallforge")], capture_output=True, timeout=60)
    if r.returncode:
        return False
    r = subprocess.run([sys.executable, "-I", "-c",
                        "import sys; sys.path.insert(0, sys.argv[1]); import wallforge.config, wallforge.paths, wallforge.updater",
                        str(ROOT)], capture_output=True, timeout=60)
    return r.returncode == 0


def update() -> str:
    """Pull the new version (git checkouts only), roll back if it does not load. Returns a short summary line."""
    if kind() != "git":
        raise RuntimeError("Not a git checkout: update it with your package manager")
    if _git("status", "--porcelain", "--untracked-files=no"):
        raise RuntimeError("Local changes in the Wallforge folder; not touching them")
    old = _git("rev-parse", "HEAD")
    _git("pull", "--ff-only", timeout=120)
    if not _healthy():
        _git("reset", "--hard", old)
        raise RuntimeError("The new version did not load, kept the old one")
    return _git("log", "-1", "--format=%h %s")


def restart() -> None:
    """Replace this process with a fresh Wallforge (same arguments)."""
    import os
    os.execv(sys.executable, [sys.executable, *sys.orig_argv[1:]])
