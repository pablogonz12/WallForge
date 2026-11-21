"""Start WallForge's session daemon (``wallforge --daemon``) at login, by every mechanism the desktops offer.

The daemon holds a lock, so several mechanisms can be on at once: the first one wins, the rest exit at once.
  * systemd user unit  - graphical-session.target (niri-session, uwsm, KDE, GNOME, ...), restarted if it dies
  * XDG autostart      - GNOME, KDE, anything with dex/xdg-autostart
  * niri               - spawn-at-startup in the managed niri/wallforge.kdl (niri ignores XDG autostart)
  * Hyprland / sway    - without systemd session integration, one line in their config (shown by --doctor)
Light module: no GTK imports, so the daemon and --doctor can use it.
"""
from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

UNIT = "wallforge.service"
CONFIG = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
XDG_FILE = CONFIG / "autostart/dev.wallforge.Wallforge-restore.desktop"
UNIT_FILE = CONFIG / "systemd/user" / UNIT
WANTS = CONFIG / "systemd/user/graphical-session.target.wants" / UNIT
PACKAGED_UNITS = ("/usr/lib/systemd/user", "/usr/local/lib/systemd/user", "/etc/systemd/user")


def launcher() -> str:
    return shutil.which("wallforge") or str(Path(__file__).resolve().parent.parent / "wallforge.sh")


def daemon_cmd() -> list[str]:
    return [launcher(), "--daemon"]


def _quote(a: str) -> str:
    return '"' + a.replace("\\", "\\\\").replace('"', '\\"') + '"' if any(c in a for c in " \t\"'\\") else a


def unit_text() -> str:
    return ("[Unit]\nDescription=WallForge live wallpapers (session daemon)\n"
            "Documentation=https://github.com/pgm1207/WallForge\n"
            "PartOf=graphical-session.target\nAfter=graphical-session.target\n\n"
            "[Service]\nType=simple\n"
            f"ExecStart={' '.join(_quote(a) for a in daemon_cmd())}\n"
            "Restart=on-failure\nRestartSec=3\n"
            # Renderers outlive the daemon (it adopts them again on start), so updating or restarting it never blinks the screen.
            "KillMode=process\n\n"
            "[Install]\nWantedBy=graphical-session.target\n")


def _systemctl(*args: str) -> subprocess.CompletedProcess | None:
    try:
        return subprocess.run(["systemctl", "--user", *args], capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.SubprocessError):
        return None


def systemd_available() -> bool:
    """A systemd user manager is running for this login."""
    if not shutil.which("systemctl"):
        return False
    r = _systemctl("is-system-running")
    return bool(r and r.stdout.strip() in ("running", "degraded", "starting"))


def packaged_unit() -> Path | None:
    return next((Path(d) / UNIT for d in PACKAGED_UNITS if (Path(d) / UNIT).is_file()), None)


def unit_enabled() -> bool:
    return WANTS.exists() or WANTS.is_symlink()


def xdg_enabled() -> bool:
    return XDG_FILE.is_file()


def desktop() -> str:
    return os.environ.get("XDG_CURRENT_DESKTOP", "").lower()


def niri_spawn() -> list[str] | None:
    return daemon_cmd() if "niri" in desktop() or os.environ.get("NIRI_SOCKET") else None


def config_hint() -> str:
    """For compositors that need a line in their own config (Hyprland, sway). Empty when none is needed or it is there."""
    cmd = " ".join(_quote(a) for a in daemon_cmd())
    d = desktop()
    for name, rel, line in (("hyprland", "hypr/hyprland.conf", f"exec-once = {cmd}"), ("sway", "sway/config", f"exec {cmd}")):
        if name in d or (name == "sway" and os.environ.get("SWAYSOCK")):
            f = CONFIG / rel
            try:
                if "wallforge" in f.read_text(errors="replace"):
                    return ""
            except OSError:
                pass
            return f"add to {f}: {line}" if not (systemd_session() and unit_enabled()) else ""
    return ""


def systemd_session() -> bool:
    """graphical-session.target is active, so the unit will really be started at login."""
    r = _systemctl("is-active", "graphical-session.target") if shutil.which("systemctl") else None
    return bool(r and r.stdout.strip() == "active")


def status() -> dict:
    from . import daemon

    return {"unit_file": UNIT_FILE.is_file() or packaged_unit() is not None, "unit_enabled": unit_enabled(),
            "xdg": xdg_enabled(), "niri": _niri_has_spawn(), "daemon": daemon.running_pid()}


def _niri_has_spawn() -> bool:
    try:
        return "spawn-at-startup" in (CONFIG / "niri/wallforge.kdl").read_text()
    except OSError:
        return False


def _write(path: Path, text: str) -> bool:
    try:
        if path.read_text() == text:
            return False
    except OSError:
        pass
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return True


def enable(overview: bool = True) -> list[str]:
    """Turn every mechanism on (idempotent). Returns what changed, for the log."""
    from . import niri

    done = []
    xdg = ("[Desktop Entry]\nType=Application\nName=WallForge (session daemon)\n"
           f"Exec={' '.join(_quote(a) for a in daemon_cmd())}\nIcon=dev.wallforge.Wallforge\nNoDisplay=true\n"
           "X-GNOME-Autostart-enabled=true\n")
    if _write(XDG_FILE, xdg):
        done.append("xdg")
    if shutil.which("systemctl"):
        # a packaged unit is used as is; otherwise write ours into the user's unit dir
        if not (packaged_unit() and daemon_cmd()[0].startswith("/usr/")):
            if _write(UNIT_FILE, unit_text()):
                done.append("unit")
                _systemctl("daemon-reload")
        if not unit_enabled():
            r = _systemctl("enable", UNIT)
            if r and r.returncode == 0:
                done.append("enable")
    err = niri.apply(overview, niri_spawn()) if niri.available() else ""
    if not err and niri.available():
        done.append("niri")
    return done


def disable(overview: bool = True):
    from . import niri

    XDG_FILE.unlink(missing_ok=True)
    if shutil.which("systemctl") and unit_enabled():
        _systemctl("disable", UNIT)
    UNIT_FILE.unlink(missing_ok=True)  # the daemon keeps running until logout; it just won't come back
    if niri.available():
        niri.apply(overview, None)


def sync(want: bool, overview: bool = True) -> list[str]:
    """Make the login mechanisms match the setting. Cheap when they already do (no subprocess)."""
    from . import niri

    if not want:
        if xdg_enabled() or unit_enabled() or _niri_has_spawn():
            disable(overview)
        return []
    if xdg_enabled() and (UNIT_FILE.is_file() or packaged_unit()) and unit_enabled() and (not niri.available() or _niri_has_spawn()):
        try:  # all there; only rewrite if the launcher moved
            if XDG_FILE.read_text().find(daemon_cmd()[0]) >= 0 and (
                    not UNIT_FILE.is_file() or UNIT_FILE.read_text() == unit_text()):
                return []
        except OSError:
            pass
    return enable(overview)
