"""What "Uninstall" means for each kind of library item (no GTK, so it is testable).

* Added to WallForge (Local, Lively, Wikimedia): the folder in the user library is deleted.
* Workshop: WallForge never downloads these itself; Steam does, through a subscription. Deleting the folder
  would only make Steam fetch it again, so the only real uninstall is to unsubscribe on the Workshop page.
"""
from __future__ import annotations

import subprocess
from dataclasses import dataclass

from . import importer, library, paths


@dataclass
class Plan:
    action: str  # "delete" | "unsubscribe" | "none"
    title: str
    message: str
    button: str


def plan(it: library.Item) -> Plan:
    if importer.is_local(it.dir):
        extra = "The original file is kept." if (it.dir / "video").is_symlink() or any(
            p.is_symlink() for p in it.dir.iterdir() if p.is_file()) else "The downloaded copy is deleted."
        return Plan("delete", f"Uninstall {it.title}?", f"It is removed from WallForge. {extra}", "Uninstall")
    wid = it.workshop_id
    if wid and any(it.dir.is_relative_to(r) for r in paths.workshop_dirs()):
        return Plan("unsubscribe", f"Unsubscribe from {it.title}?",
                    "This wallpaper is managed by Steam: deleting its folder would only make Steam download it again. "
                    "WallForge opens its Workshop page; click Unsubscribe there and Steam removes the files.", "Open Workshop page")
    return Plan("none", it.title, "This wallpaper is not managed by WallForge.", "")


def steam_url(wid: str) -> str:
    return f"steam://url/CommunityFilePage/{wid}"


def run(it: library.Item, opener=None) -> str:
    """Carry the plan out. Returns "deleted" or "unsubscribe". ``opener`` is injectable for tests."""
    p = plan(it)
    if p.action == "delete":
        importer.remove(it.dir)
        return "deleted"
    if p.action == "unsubscribe":
        (opener or _open)(steam_url(it.workshop_id))
        return "unsubscribe"
    raise importer.ImportError_("This wallpaper cannot be uninstalled from WallForge")


def _open(url: str):
    subprocess.Popen(["steam", "-ifrunning", url], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True)
