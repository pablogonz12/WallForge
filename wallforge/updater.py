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
    _rebuild_render(old)
    return _git("log", "-1", "--format=%h %s")


def _rebuild_render(old: str) -> None:
    """If the native renderer was built and its sources changed, rebuild it (best effort, keeps the old binary on failure)."""
    import shutil
    crate = ROOT / "render"
    if not (crate / "target/release/wallforge-render").is_file() or not shutil.which("cargo"):
        return
    try:
        if not _git("diff", "--name-only", old, "HEAD", "--", "render"):
            return
        subprocess.run(["cargo", "build", "--release", "-q"], cwd=str(crate), capture_output=True, timeout=900)
    except Exception:
        pass


def restart() -> None:
    """Replace this process with a fresh Wallforge (same arguments)."""
    import os
    os.execv(sys.executable, [sys.executable, *sys.orig_argv[1:]])


BUILD_ERROR = ""  # last line of the renderer build's error output, for the UI


def ensure_render() -> bool:
    """Git checkout without a built renderer: build it once in the background (needs cargo). True if now available."""
    import shutil
    crate = ROOT / "render"
    if (crate / "target/release/wallforge-render").is_file():
        return True
    if not (crate / "Cargo.toml").is_file() or not shutil.which("cargo"):
        return False
    global BUILD_ERROR
    try:
        r = subprocess.run(["cargo", "build", "--release", "-q"], cwd=str(crate), capture_output=True, text=True, timeout=1200)
        if r.returncode:
            lines = [l.strip() for l in (r.stderr or "").splitlines() if l.strip()]
            err = next((l for l in lines if l.startswith("error")), lines[-1] if lines else "build failed")
            BUILD_ERROR = err[:160]
        return r.returncode == 0
    except Exception as e:
        BUILD_ERROR = str(e)[:160]
        return False
