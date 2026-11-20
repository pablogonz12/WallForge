#!/usr/bin/env python3
"""Steam Workshop browser for Wallpaper Engine (appid 431960). stdlib only.
The browse page is a React SSR page; items are embedded as JSON in
<script id="valve-ssr-data">. No credentials needed.
Usage: workshop_browse.py [query] [--sort trend|mostrecent|totaluniquesubscribers|textsearch]
       [--page N] [--tag Scene --tag Video] [--period 7] [--details ID ...]
"""
import json, re, sys, time, argparse, urllib.request, urllib.parse

APPID = 431960
UA = "Mozilla/5.0 (X11; Linux x86_64) wallforge/0.1"
# sort values: trend (needs days=), mostrecent, totaluniquesubscribers, lastupdated, textsearch
TAG_TYPES = ["Scene", "Video", "Web", "Application"]


def _get(url, data=None):
    req = urllib.request.Request(url, data=data, headers={"User-Agent": UA, "Accept-Language": "en"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read().decode("utf-8", "replace")


def _find_results(obj):
    if isinstance(obj, dict):
        if isinstance(obj.get("results"), list) and obj["results"] and "publishedfileid" in obj["results"][0]:
            return obj
        for v in obj.values():
            r = _find_results(v)
            if r:
                return r
    elif isinstance(obj, list):
        for v in obj:
            r = _find_results(v)
            if r:
                return r
    elif isinstance(obj, str) and obj[:1] in "{[" and "publishedfileid" in obj:
        try:
            return _find_results(json.loads(obj))
        except ValueError:
            pass
    return None


ULTRAWIDE_TAGS = ["Ultrawide 3440 x 1440", "Ultrawide 2560 x 1080", "Dual 5120 x 1440", "Dual 7680 x 2160",
                  "Dual 3840 x 1080", "Triple 7680 x 1440"]
MATURE_TAGS = ["Mature", "Questionable"]


def browse(query="", sort="trend", page=1, tags=(), days=7, section="readytouseitems", exclude=()):
    if query and sort == "trend":
        sort = "textsearch"
    params = [("appid", APPID), ("browsesort", sort), ("section", section), ("p", page),
              ("actualsort", sort), ("childpublishedfileid", 0)]
    if sort == "trend":
        params.append(("days", days))
    if query:
        params.append(("searchtext", query))
    for t in tags:
        params.append(("requiredtags[]", t))
    for t in exclude:
        params.append(("excludedtags[]", t))
    html = _get("https://steamcommunity.com/workshop/browse/?" + urllib.parse.urlencode(params))
    m = re.search(r'<script type="application/json" id="valve-ssr-data"[^>]*>(.*?)</script>', html, re.S)
    if not m:
        raise RuntimeError("ssr data not found")
    data = json.loads(m.group(1))
    found = None
    found = _find_results(data.get("renderContext")) or _find_results(data.get("loaderData"))
    if not found:
        return {"total": 0, "pages": 0, "items": []}
    items = []
    for r in found["results"]:
        tags_ = [t["tag"] for t in r.get("tags", [])]
        items.append({
            "id": r["publishedfileid"],
            "title": r.get("title", ""),
            "preview": r.get("preview_url", ""),
            "author_steamid": r.get("creator", ""),
            "stars": r.get("star_rating"),
            "votes": r.get("total_votes"),
            "subs": r.get("subscriptions"),
            "size": int(r.get("file_size") or 0),
            "type": next((t for t in tags_ if t in TAG_TYPES), None),
            "tags": tags_,
            "desc": r.get("short_description", ""),
            "url": item_url(r["publishedfileid"]),
        })
    return {"total": found.get("total_count"), "pages": found.get("total_pages"), "items": items}


def item_url(item_id) -> str:
    """The Workshop page of a wallpaper: the link to share."""
    return "https://steamcommunity.com/sharedfiles/filedetails/?id=" + str(item_id)


_LINK = re.compile(r"(?:steamcommunity\.com/(?:sharedfiles|workshop)/filedetails/?\?(?:[^\s#]*?&)?id=|CommunityFilePage/)(\d+)")


def parse_item_id(text: str):
    """Workshop id from a pasted Workshop link (or a bare id), else None."""
    t = (text or "").strip()
    if re.fullmatch(r"\d{7,12}", t):
        return t
    m = _LINK.search(t)
    return m.group(1) if m else None


def lookup(item_id):
    """One Workshop item as a browse() entry, or None when it is not a Wallpaper Engine wallpaper."""
    d = details([str(item_id)])
    r = d[0] if d else {}
    if r.get("result") != 1 or int(r.get("consumer_app_id") or 0) != APPID:
        return None
    tags_ = [t["tag"] for t in r.get("tags", []) if isinstance(t, dict) and "tag" in t]
    return {"id": r["publishedfileid"], "title": r.get("title", ""), "preview": r.get("preview_url", ""),
            "author_steamid": r.get("creator", ""), "stars": None, "votes": None, "subs": r.get("subscriptions"),
            "size": int(r.get("file_size") or 0), "type": next((t for t in tags_ if t in TAG_TYPES), None), "tags": tags_,
            "desc": r.get("description", "")[:300], "url": item_url(r["publishedfileid"])}


def details(ids):
    """ISteamRemoteStorage/GetPublishedFileDetails, POST, no key."""
    body = {"itemcount": len(ids)}
    for i, x in enumerate(ids):
        body["publishedfileids[%d]" % i] = x
    out = _get("https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/",
               urllib.parse.urlencode(body).encode())
    return json.loads(out)["response"]["publishedfiledetails"]


# ---- download through the running Steam client ---------------------------------
import asyncio
import os
import subprocess
from pathlib import Path

from . import library, paths


def installed(item_id: str) -> bool:
    return any((d / str(item_id) / "project.json").is_file() for d in paths.workshop_dirs())


def open_in_steam(item_id: str):
    subprocess.Popen(["steam", "-ifrunning", f"steam://url/CommunityFilePage/{item_id}"],
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


def _cdp_port() -> int | None:
    """Steam's embedded browser exposes a debug port; read it from the process args."""
    for p in Path("/proc").glob("[0-9]*/cmdline"):
        try:
            args = p.read_bytes().split(b"\0")
        except Exception:
            continue
        if b"steamwebhelper" in (args[0] if args else b""):
            for a in args:
                if a.startswith(b"--remote-debugging-port="):
                    return int(a.split(b"=")[1])
    return None


async def _cdp_subscribe(port: int, item_id: str) -> str:
    import aiohttp
    async with aiohttp.ClientSession() as s:
        async with s.get(f"http://127.0.0.1:{port}/json") as r:
            tabs = await r.json(content_type=None)
        tab = next((t for t in tabs if item_id in t.get("url", "") and t.get("webSocketDebuggerUrl")), None)
        if not tab:
            return "page-not-open"
        async with s.ws_connect(tab["webSocketDebuggerUrl"]) as ws:
            js = ("(function(){var b=document.getElementById('SubscribeItemBtn')||"
                  "document.querySelector('a[onclick*=Subscribe]');"
                  "if(!b)return 'no-button';"
                  "if(/subscribed/i.test(b.className+b.innerText)&&!/^\\s*subscribe/i.test(b.innerText))return 'already';"
                  "b.click();return 'clicked';})()")
            await ws.send_json({"id": 1, "method": "Runtime.evaluate", "params": {"expression": js, "returnByValue": True}})
            async for m in ws:
                d = m.json()
                if d.get("id") == 1:
                    return str(d.get("result", {}).get("result", {}).get("value", "error"))
    return "error"


def subscribe(item_id: str) -> str:
    """Open the item in Steam and click Subscribe there. Returns a short status string.
    Falls back to 'manual' (page open, user clicks) when Steam's debug port is unavailable."""
    open_in_steam(item_id)
    port = _cdp_port()
    if not port:
        return "manual"
    import time
    for _ in range(12):
        time.sleep(1)
        try:
            r = asyncio.run(_cdp_subscribe(port, item_id))
        except Exception:
            r = "error"
        if r != "page-not-open":
            return r if r in ("clicked", "already") else "manual"
    return "manual"
