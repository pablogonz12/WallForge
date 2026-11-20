"""Picking the next wallpaper for the rotation timer (pure logic, no GTK)."""
from __future__ import annotations

import random


def candidates(items, *, hidden=lambda _item: False, favourites=(), only_favourites=False, web_missing=False):
    """Wallpapers rotation may pick, in library order.

    `hidden` is the same filter the library view uses (content settings, ultrawide on a 16:9 screen);
    `web_missing` is true when the WebKit packages are missing, so web wallpapers cannot play.
    """
    fav = set(favourites or ())
    pool = []
    for item in items:
        if item.broken or hidden(item):
            continue
        if item.kind == "web" and web_missing:
            continue
        if only_favourites and item.id not in fav:
            continue
        pool.append(item)
    return pool


def next_item(pool, current_ids=(), *, shuffle=True, rng=random):
    """The wallpaper to show now, or None for an empty pool.

    `current_ids` are what the screens show: they are skipped, unless they are all there is.
    With `shuffle` off the next wallpaper in library order is returned, wrapping at the end.
    """
    if not pool:
        return None
    cur = {str(i) for i in current_ids if i}
    usable = [i for i in pool if i.id not in cur] or pool
    if shuffle:
        return rng.choice(usable)
    ids = [i.id for i in pool]
    start = max((ids.index(x) for x in cur if x in ids), default=-1)
    for step in range(1, len(pool) + 1):
        item = pool[(start + step) % len(pool)]
        if item.id not in cur:
            return item
    return pool[(start + 1) % len(pool)]
