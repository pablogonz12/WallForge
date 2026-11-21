"""Pause the renderer of an output that a fullscreen window completely covers, resume it the moment it is uncovered.

niri keeps sending frame callbacks to a covered wallpaper, so it would keep decoding and drawing for nothing. The renderer
pauses on SIGUSR2 and resumes on SIGUSR1 (render/src/main.rs). On niri this is event driven: a thread follows
``niri msg -j event-stream`` and re-evaluates on window/workspace/overview events (first event at once, bursts debounced).

Rules that keep it safe:
  * only the visible renderer of an output (its pidfile pid) is signalled, never a preloaded spare: SIGUSR1 also clears a
    spare's --hidden flag, so a spare must never get one;
  * only native renderers (the web player and linux-wallpaperengine would die on SIGUSR2) and only after their first frame;
  * the pause state is kept per pid, so a renderer started while the output is covered (gapless switch, crash restart) is
    paused by the next ``apply``;
  * while niri's overview is open the wallpaper is visible behind it, so nothing counts as covered.
"""
from __future__ import annotations

import json
import logging
import os
import select
import shutil
import signal
import subprocess
import threading
import time

from . import engine, occlusion

log = logging.getLogger("wallforge.daemon")
DEBOUNCE = 0.1
SETTLE = 0.45
# events that cannot change what is visible
_IGNORED = ("KeyboardLayout", "ConfigLoaded", "ScreenshotCaptured", "CastsChanged", "CastStartedOrChanged", "CastStopped")


class Pauser:
    def __init__(self, eng=engine, send=os.kill, covered_fn=occlusion.covered_outputs):
        self.eng, self.send, self.covered_fn = eng, send, covered_fn
        self.lock = threading.Lock()
        self.covered: set[str] = set()
        self.overview = False
        self.paused: dict[str, int] = {}  # output -> pid we paused
        self.enabled = True
        self.thread: threading.Thread | None = None

    # ------------------------------------------------------------ decisions (testable)
    def wanted(self) -> set[str]:
        return set() if (self.overview or not self.enabled) else set(self.covered)

    def apply(self, outputs: list[str]):
        """Bring every output's renderer in line with the covered set."""
        with self.lock:
            want = self.wanted()
            for name in outputs:
                pid = self.eng.visible_pid(name)
                if pid is None:
                    self.paused.pop(name, None)
                    continue
                was = self.paused.get(name)
                if name in want and was != pid and self.eng.pausable(name):
                    if self._signal(pid, signal.SIGUSR2):
                        self.paused[name] = pid
                        log.info("%s: covered, paused renderer %d", name, pid)
                elif name not in want and was is not None:
                    if was == pid:
                        self._signal(pid, signal.SIGUSR1)
                        log.info("%s: visible again, resumed renderer %d", name, pid)
                    self.paused.pop(name, None)  # (a different pid is a fresh, unpaused renderer)

    def _signal(self, pid: int, sig) -> bool:
        try:
            self.send(pid, sig)
            return True
        except (ProcessLookupError, PermissionError):
            return False

    def resume_all(self):
        with self.lock:
            for name, pid in list(self.paused.items()):
                self._signal(pid, signal.SIGUSR1)
            self.paused.clear()

    # ------------------------------------------------------------ niri event stream
    def start(self, outputs_fn):
        if self.thread or not (os.environ.get("NIRI_SOCKET") and shutil.which("niri")):
            return
        self.thread = threading.Thread(target=self._follow, args=(outputs_fn,), daemon=True, name="wf-occlusion")
        self.thread.start()

    def _refresh(self, outputs_fn):
        try:
            cov = self.covered_fn()
        except Exception as e:
            log.debug("covered check failed: %s", e)
            return
        with self.lock:
            if cov != self.covered:
                log.info("covered outputs: %s", sorted(cov))
            self.covered = cov
        self.apply(outputs_fn())

    def _follow(self, outputs_fn):
        while True:
            try:
                p = subprocess.Popen(["niri", "msg", "-j", "event-stream"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            except OSError:
                return
            try:
                self._refresh(outputs_fn)
                state = 0  # 0 idle, 1 burst (wait for quiet), 2 settle (layout animations end without an event)
                while True:
                    ready, _, _ = select.select([p.stdout], [], [], (None, DEBOUNCE, SETTLE)[state])
                    if not ready:
                        self._refresh(outputs_fn)
                        state = 2 if state == 1 else 0
                        continue
                    line = p.stdout.readline()
                    if not line:
                        break  # niri went away
                    ev = _kind(line)
                    if ev is None or ev.startswith(_IGNORED):
                        continue
                    if ev == "OverviewOpenedOrClosed":
                        try:
                            with self.lock:
                                self.overview = bool(json.loads(line)[ev]["is_open"])
                                log.info("overview %s", "open" if self.overview else "closed")
                        except (ValueError, KeyError):
                            pass
                        self.apply(outputs_fn())  # resume at once, no IPC round trip
                    if state == 0:  # leading edge: act now, then debounce the rest of the burst
                        self._refresh(outputs_fn)
                    state = 1
            except Exception:
                log.exception("event stream failed")
            finally:
                try:
                    p.kill()
                    p.wait(timeout=2)
                except Exception:
                    pass
            time.sleep(2)  # niri restarted or the stream broke: reconnect


def _kind(line: str) -> str | None:
    try:
        d = json.loads(line)
        return next(iter(d)) if isinstance(d, dict) and d else None
    except ValueError:
        return None
