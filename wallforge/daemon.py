"""``wallforge --daemon``: the session supervisor that makes wallpapers come back at every login.

It follows config.json (the truth) and the compositor's outputs:
  * starts every assigned output in parallel, adopts renderers that already run (GUI-started, or before a daemon restart)
  * matches assignments by connector *and* monitor identity, so docks that renumber DP-3/DP-5 do not lose them
  * restarts a crashed renderer with backoff, retries outputs that were not ready yet, follows hotplug
  * stacking guard (niri, Hyprland): if another client's background surface (Noctalia, swaybg, ...) got created after ours
    and hides it, recreate ours on top
It never resurrects an output the user stopped in the GUI, and it holds a lock, so any number of autostart mechanisms
can start it. Light imports only: no GTK/Adw unless the Gdk fallback is needed to list monitors.
"""
from __future__ import annotations

import fcntl
import glob
import json
import logging
import logging.handlers
import os
import select
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

from . import config, engine, library, monitors, pauser

log = logging.getLogger("wallforge.daemon")
RUNTIME = Path(os.environ.get("XDG_RUNTIME_DIR") or "/tmp")
LOCK = engine.STATE / "daemon.lock"
FAST, SLOW = 0.5, 5.0  # tick seconds: right after a start or change, and when idle
FAST_FOR = 45.0
CRASH_LIMIT, CRASH_WINDOW, GIVE_UP_FOR = 5, 60.0, 600.0
GUARD_GAP, GUARD_LIMIT, GUARD_WINDOW = 15.0, 6, 600.0
WAIT_DISPLAY = 120.0

_lock_fh = None
_wake_keep: list = []


# ---------------------------------------------------------------- single instance
def acquire_lock() -> bool:
    global _lock_fh
    engine.STATE.mkdir(parents=True, exist_ok=True)
    fh = open(LOCK, "a+")
    try:
        fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        fh.close()
        return False
    fh.seek(0)
    fh.truncate()
    fh.write(str(os.getpid()))
    fh.flush()
    _lock_fh = fh
    return True


def running_pid() -> int | None:
    """PID of the daemon, or None when no daemon holds the lock."""
    try:
        fh = open(LOCK, "a+")
    except OSError:
        return None
    try:
        try:
            fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            fh.seek(0)
            try:
                return int(fh.read().strip() or 0) or -1
            except ValueError:
                return -1
        fcntl.flock(fh, fcntl.LOCK_UN)
        return None
    finally:
        fh.close()


# ---------------------------------------------------------------- environment (units may start before the session env is imported)
def discover_env():
    """Fill in the compositor sockets when the daemon was started without a full session environment."""
    if not os.environ.get("XDG_RUNTIME_DIR"):
        os.environ["XDG_RUNTIME_DIR"] = f"/run/user/{os.getuid()}"
    rt = os.environ["XDG_RUNTIME_DIR"]
    if not os.environ.get("WAYLAND_DISPLAY"):
        socks = sorted(p for p in glob.glob(f"{rt}/wayland-*") if not p.endswith(".lock"))
        if socks:
            os.environ["WAYLAND_DISPLAY"] = os.path.basename(socks[0])
    if not os.environ.get("NIRI_SOCKET"):
        socks = sorted(glob.glob(f"{rt}/niri.*.sock"))
        if socks:
            os.environ["NIRI_SOCKET"] = socks[-1]
    if not os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"):
        inst = sorted(glob.glob(f"{rt}/hypr/*/.socket.sock"))
        if inst:
            os.environ["HYPRLAND_INSTANCE_SIGNATURE"] = os.path.basename(os.path.dirname(inst[-1]))
    if not os.environ.get("SWAYSOCK"):
        socks = sorted(glob.glob(f"{rt}/sway-ipc.*.sock"))
        if socks:
            os.environ["SWAYSOCK"] = socks[-1]


# ---------------------------------------------------------------- identity matching (pure)
def resolve_state(saved: dict, displaced: list, live: list[dict]) -> tuple[dict, dict, list]:
    """Resolve connector assignments and displaced records without losing unplugged monitors.

    Displaced records retain their last connector and complete entry, but are not
    exposed as connector keys to app/CLI readers. Connector matches take priority,
    followed by identity matches; each entry and live output is used at most once.
    """
    entries = [(k, dict(v), False) for k, v in saved.items() if isinstance(v, dict)]
    invalid = []
    for record in displaced:
        if (isinstance(record, dict) and isinstance(record.get("connector"), str)
                and isinstance(record.get("entry"), dict)):
            entries.append((record["connector"], dict(record["entry"]), record))
        else:
            invalid.append(record)
    match: dict[str, int] = {}
    taken: set[int] = set()
    for o in live:
        for i, (name, entry, _) in enumerate(entries):
            if (i not in taken and name == o["name"] and
                    (not entry.get("ident") or not o.get("ident") or entry["ident"] == o["ident"])):
                match[o["name"]] = i
                taken.add(i)
                break
    for o in live:
        if o["name"] in match or not o.get("ident"):
            continue
        for i, (_, entry, _) in enumerate(entries):
            if i not in taken and entry.get("ident") == o["ident"]:
                match[o["name"]] = i
                taken.add(i)
                break
    out = {}
    for o in live:
        i = match.get(o["name"])
        if i is not None:
            entry = entries[i][1]
            if o.get("ident"):
                entry["ident"] = o["ident"]
            out[o["name"]] = entry
    names = {o["name"] for o in live}
    dormant = list(invalid)
    for i, (name, entry, record) in enumerate(entries):
        if i in taken:
            continue
        if record is not False:
            dormant.append(record)
        elif name in names or name in out:
            dormant.append({"connector": name, "entry": entry})
        else:
            out[name] = entry
    return {name: entries[i][1] for name, i in match.items()}, out, dormant


# ---------------------------------------------------------------- stacking guard
def layers_snapshot() -> list[tuple[str, str, str]] | None:
    """[(namespace, output, layer)] in stacking order (later = above), or None where the compositor has no layer IPC."""
    try:
        if os.environ.get("NIRI_SOCKET") and shutil.which("niri"):
            r = subprocess.run(["niri", "msg", "-j", "layers"], capture_output=True, text=True, timeout=3)
            return [(x.get("namespace", ""), x.get("output", ""), x.get("layer", "")) for x in json.loads(r.stdout)]
        if os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") and shutil.which("hyprctl"):
            r = subprocess.run(["hyprctl", "-j", "layers"], capture_output=True, text=True, timeout=3)
            names = {"0": "Background", "1": "Bottom", "2": "Top", "3": "Overlay"}
            res = []
            for out, v in json.loads(r.stdout).items():
                for lvl, lst in (v.get("levels") or {}).items():
                    res += [(x.get("namespace", ""), out, names.get(str(lvl), str(lvl))) for x in lst]
            return res
    except Exception as e:
        log.debug("layer snapshot failed: %s", e)
    return None


def covered(layers: list[tuple[str, str, str]], output: str) -> str:
    """Namespace of a foreign background surface stacked above ours on this output, else ''."""
    bg = [ns for ns, out, layer in layers if out == output and layer.lower() == "background"]
    ours = [i for i, ns in enumerate(bg) if ns == "wallforge"]
    if not ours:
        return ""
    return next((ns for ns in bg[ours[-1] + 1:] if not ns.startswith("wallforge")), "")


# ---------------------------------------------------------------- supervisor
class Supervisor:
    """All decisions live here; ``tick`` is called with the current facts so tests can drive it with a fake engine."""

    def __init__(self, eng=engine, clock=time.monotonic):
        self.eng, self.now = eng, clock
        self.starts: dict[str, list[float]] = {}  # output -> recent start times
        self.next_try: dict[str, float] = {}
        self.gave_up: dict[str, float] = {}  # output -> retry-after
        self.recreated: dict[str, list[float]] = {}
        self.guard_off: dict[str, float] = {}
        self.began: dict[str, float] = {}  # output -> last start, the guard waits for the surface to appear
        self.items: dict[str, library.Item] = {}
        self._scanned = -1e9
        self.busy_until = 0.0  # fast ticking until

    def _item(self, iid: str):
        it = self.items.get(iid)
        if it is None and self.now() - self._scanned > 30:  # unknown id: the library may have changed
            self.items = {i.id: i for i in library.scan()}
            self._scanned = self.now()
            it = self.items.get(iid)
        return it

    def _start(self, cfg, name: str, iid: str, why: str) -> bool:
        it = self._item(iid)
        if not it or it.broken:
            return False
        try:
            self.eng.start(cfg, it, name)
        except Exception as e:
            log.warning("%s: start failed: %s", name, e)
            return False
        self.began[name] = self.now()
        self.busy_until = self.now() + FAST_FOR
        log.info("%s: started %s (%s)", name, iid, why)
        return True

    def tick(self, cfg, live: list[dict], layers) -> None:
        now = self.now()
        saved = cfg.data.get("outputs", {})
        match, _, _ = resolve_state(saved, cfg.data.get("displaced_outputs", []), live)
        paused = self.eng.pause_for_battery(cfg)
        for o in live:
            name = o["name"]
            e = match.get(name)
            iid = (e or {}).get("id")
            if not iid or paused or self.eng.is_stopped(name) or self.eng.busy(name):
                continue
            if not self.eng.alive_fast(name):
                if now < self.gave_up.get(name, 0) or now < self.next_try.get(name, 0):
                    continue
                hist = [t for t in self.starts.get(name, []) if now - t < CRASH_WINDOW]
                if len(hist) >= CRASH_LIMIT:
                    self.gave_up[name] = now + GIVE_UP_FOR
                    self.starts[name] = []
                    log.error("%s: renderer keeps dying (%d starts in %ds); giving up for %d min. See %s",
                              name, len(hist), CRASH_WINDOW, GIVE_UP_FOR // 60, engine.STATE / f"{name}.log")
                    continue
                why = "first start" if not hist else f"restart #{len(hist)}"
                if self._start(cfg, name, iid, why):
                    hist.append(now)
                    # the first crash is restarted at once; backoff only when it keeps dying
                    k = len(hist)
                    self.next_try[name] = now + (0 if k <= 1 else min(8.0, 0.5 * 2 ** (k - 2)))
                self.starts[name] = hist
                continue
            if now - self.began.get(name, 0) > CRASH_WINDOW:  # it has been stable: forget old crashes
                self.starts.pop(name, None)
            if layers is not None:
                self._guard(cfg, name, iid, layers, now)
        # unplugged: stop its renderer (the saved assignment stays)
        if live:
            names = {o["name"] for o in live}
            for name in list(saved):
                if name not in names and self.eng.alive_fast(name):
                    self.eng.stop(name)
                    log.info("%s: output gone, stopped its renderer", name)

    def wait_hint(self, cfg, live: list[dict], normal: float) -> float:
        """How long the loop may sleep: short while a wanted renderer is down and due (or about to be), else ``normal``."""
        now = self.now()
        saved = cfg.data.get("outputs", {})
        match, _, _ = resolve_state(saved, cfg.data.get("displaced_outputs", []), live)
        hint = normal
        for o in live:
            name = o["name"]
            if not (match.get(name) or {}).get("id") or self.eng.is_stopped(name) or self.eng.pause_for_battery(cfg):
                continue
            if self.eng.alive_fast(name):
                continue
            due = max(self.gave_up.get(name, 0), self.next_try.get(name, 0))
            hint = min(hint, max(0.05, due - now))
        return hint

    def _guard(self, cfg, name: str, iid: str, layers, now: float):
        if self.eng.current_layer(name) != "background" or now - self.began.get(name, 0) < 1.0:
            return
        above = covered(layers, name)
        if not above or now < self.guard_off.get(name, 0):
            return
        hist = [t for t in self.recreated.get(name, []) if now - t < GUARD_WINDOW]
        if hist and now - hist[-1] < GUARD_GAP:
            return
        if len(hist) >= GUARD_LIMIT:
            self.guard_off[name] = now + GUARD_WINDOW
            log.error("%s: %s keeps covering the wallpaper; stacking guard paused for %d min", name, above, GUARD_WINDOW // 60)
            self.recreated[name] = []
            return
        if self._start(cfg, name, iid, f"stacking: {above} was above"):
            hist.append(now)
        self.recreated[name] = hist


# ---------------------------------------------------------------- main loop
def _setup_logging():
    d = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "wallforge"
    d.mkdir(parents=True, exist_ok=True)
    fmt = logging.Formatter("%(asctime)s.%(msecs)03d %(levelname)s %(message)s", "%Y-%m-%d %H:%M:%S")
    fh = logging.handlers.RotatingFileHandler(d / "daemon.log", maxBytes=200_000, backupCount=1, encoding="utf-8")
    sh = logging.StreamHandler()
    for h in (fh, sh):
        h.setFormatter(fmt)
        log.addHandler(h)
    log.setLevel(logging.INFO)


def _child_wakeup() -> socket.socket:
    """A socket that becomes readable when a child exits (SIGCHLD): renderers we started wake the loop at once."""
    r, w = socket.socketpair()
    r.setblocking(False)
    w.setblocking(False)
    signal.signal(signal.SIGCHLD, lambda *_: None)  # a handler must exist for the wakeup fd to be written
    signal.set_wakeup_fd(w.fileno(), warn_on_full_buffer=False)
    _wake_keep.append(w)  # the write end must stay open for the life of the process
    return r


def _drain(r: socket.socket):
    try:
        while r.recv(4096):
            pass
    except (BlockingIOError, OSError):
        pass


def _pidfds(live: list[dict]) -> list[int]:
    """pidfds of the running renderers: readable the moment one exits, child of ours or not."""
    fds = []
    for o in live:
        try:
            fds.append(os.pidfd_open(int((engine.STATE / f"{o['name']}.pid").read_text())))
        except (OSError, ValueError, AttributeError):
            pass
    return fds


def _reap():
    try:
        while os.waitpid(-1, os.WNOHANG)[0]:
            pass
    except ChildProcessError:
        pass


def _persist(live: list[dict]):
    """Write idents / re-keyed connectors back, from a fresh read so a GUI save in between is not lost."""
    with config.output_lock():
        fresh = config.Config()
        _, fixed, displaced = resolve_state(fresh.data.get("outputs", {}), fresh.data.get("displaced_outputs", []), live)
        if fixed != fresh.data.get("outputs") or displaced != fresh.data.get("displaced_outputs", []):
            fresh.data["outputs"] = fixed
            fresh.data["displaced_outputs"] = displaced
            fresh.save()
            log.info("saved outputs: %s", {k: v.get("ident", "") for k, v in fixed.items()})


def main() -> int:
    if not acquire_lock():
        print("wallforge daemon already running", file=sys.stderr)
        return 0
    _setup_logging()
    discover_env()
    log.info("daemon started (pid %d, %s)", os.getpid(), os.environ.get("XDG_CURRENT_DESKTOP", "?"))
    sup = Supervisor()
    wake = _child_wakeup()
    pz = pauser.Pauser()
    names_now = lambda: [o["name"] for o in (last_live or [])]  # noqa: E731
    cfg, mtime = config.Config(), 0.0
    t0, seen_outputs, empty_since = time.monotonic(), False, None
    last_live: list[dict] | None = None
    while True:
        _reap()
        try:
            m = config.CONFIG_FILE.stat().st_mtime
            if m != mtime:
                cfg, mtime = config.Config(), m
                sup.busy_until = max(sup.busy_until, time.monotonic() + FAST_FOR)
        except OSError:
            pass
        live = monitors.outputs()
        if live:
            seen_outputs, empty_since = True, None
            live.sort(key=lambda o: o["name"])
            if [(o["name"], o.get("ident")) for o in live] != [(o["name"], o.get("ident")) for o in (last_live or [])]:
                log.info("outputs: %s", [(o["name"], o.get("ident", "")) for o in live])
                sup.busy_until = max(sup.busy_until, time.monotonic() + FAST_FOR)
                last_live = live
                try:
                    _persist(live)
                    cfg, mtime = config.Config(), config.CONFIG_FILE.stat().st_mtime
                except Exception as e:
                    log.warning("could not save outputs: %s", e)
            try:
                sup.tick(cfg, live, layers_snapshot())
            except Exception:
                log.exception("tick failed")
        else:
            now = time.monotonic()
            empty_since = empty_since or now
            if (not seen_outputs and now - t0 > WAIT_DISPLAY) or (seen_outputs and now - empty_since > 60):
                log.info("no display (any more); exiting")
                return 0
        want_pause = bool(cfg.data["engine"].get("pause_when_covered", True))
        if want_pause != pz.enabled:
            pz.enabled = want_pause
        if want_pause:
            pz.start(names_now)
        pz.apply([o["name"] for o in live])  # new pids (gapless switch, restart) get the current state
        normal = FAST if time.monotonic() < sup.busy_until or not seen_outputs else SLOW
        # Forget the SIGCHLDs caused by our own `niri msg` calls, then look once more: a renderer that died in between is
        # caught here, one that dies later wakes the select below.
        _drain(wake)
        _reap()
        timeout = sup.wait_hint(cfg, live, normal) if live else normal
        fds = _pidfds(live)  # renderers we did not start (GUI, before a daemon restart) wake us too
        try:
            select.select([wake, *fds], [], [], timeout)
        finally:
            for fd in fds:
                os.close(fd)
