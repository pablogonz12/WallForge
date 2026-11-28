"""Wikimedia Commons as a Discover source: free videos (WebM/Ogv) with per-file licences. stdlib only.

No key needed. Etiquette: a descriptive generic User-Agent, ``maxlag=5``, small pages, one request at a time,
and only files under licences that allow reuse (public domain, CC0, CC BY, CC BY-SA). The licence and the
author are shown in the UI and written to the project, because CC BY / BY-SA require attribution.
"""
from __future__ import annotations

import html
import json
import re
import shutil
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from . import importer, paths

UA = "WallForge/0.8 (https://github.com/pgm1207/WallForge)"
API = "https://commons.wikimedia.org/w/api.php"
SOURCE = "Wikimedia"
PAGE_SIZE = 24  # raw search hits per page; portrait/low-res ones are filtered out afterwards
MIN_WIDTH = 1280
MIN_RATIO = 1.4  # landscape only
MIN_SECONDS = 3
MAX_DOWNLOAD = 500 * 1024 ** 2

# Dropdown label -> Commons category, searched with deepcat: so files filed in subcategories
# ("Videos of waterfalls in Norway") are found too
TOPICS = {
    "Nature": "Videos of nature",
    "Time-lapse": "Time-lapse videos",
    "Clouds": "Videos of clouds",
    "Waterfalls": "Videos of waterfalls",
    "Forests": "Videos of forests",
    "Mountains": "Videos of mountains",
    "Oceans": "Videos of oceans",
    "Beaches": "Videos of beaches",
    "Lakes": "Videos of lakes",
    "Rivers": "Videos of rivers",
    "Rain": "Videos of rain",
    "Snow": "Videos of snow",
    "Fire": "Videos of fire",
    "Sunsets": "Videos of sunsets",
    "Night": "Videos of night",
    "Earth from space": "Videos of Earth from space",
    "Cities": "Videos of cities",
    "Fish": "Videos of fish",
}

# Licence names (LicenseShortName) that may be redistributed with attribution
_OK = re.compile(r"^(public domain|pd\b|cc0|cc[ -]by(-sa)?\b|cc[ -]zero)", re.I)
_NC = re.compile(r"\b(nc|nd)\b|non-?commercial|no ?derivs", re.I)


def licence_ok(name: str) -> bool:
    return bool(name) and bool(_OK.match(name.strip())) and not _NC.search(name)


def _text(s) -> str:
    """HTML fragment from extmetadata -> plain text."""
    s = re.sub(r"<[^>]+>", "", str(s or ""))
    return re.sub(r"\s+", " ", html.unescape(s)).strip()


def _get(params: dict) -> dict:
    q = dict(params, format="json", formatversion="2", maxlag="5")
    url = API + "?" + urllib.parse.urlencode(q)
    for attempt in range(2):
        req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                data = json.loads(r.read().decode("utf-8", "replace"))
        except urllib.error.HTTPError as e:
            if e.code in (429, 503) and attempt == 0:
                time.sleep(min(int(e.headers.get("Retry-After") or 5), 10))
                continue
            raise
        err = data.get("error")
        if err and err.get("code") == "maxlag" and attempt == 0:  # servers are busy: wait once, as asked
            time.sleep(5)
            continue
        if err:
            raise RuntimeError(err.get("info", "Commons error"))
        return data
    raise RuntimeError("Commons is busy, try again in a moment")


def best_source(info: dict) -> dict | None:
    """The file to download: the largest WebM transcode up to 1080p, else the original when it is WebM/MP4."""
    cands = [d for d in info.get("derivatives", []) if str(d.get("type", "")).startswith("video/webm")
             and 720 <= int(d.get("height") or 0) <= 1080 and d.get("src")]
    if cands:
        d = max(cands, key=lambda x: x["height"])
        secs = float(info.get("duration") or 0)
        return {"url": d["src"], "width": d["width"], "height": d["height"], "ext": ".webm",
                "size": int(d.get("bandwidth", 0) / 8 * secs) or None}
    if info.get("mime") in ("video/webm", "video/mp4") and int(info.get("height") or 0) <= 1440 and info.get("url"):
        ext = ".webm" if info["mime"] == "video/webm" else ".mp4"
        return {"url": info["url"], "width": info["width"], "height": info["height"], "ext": ext, "size": info.get("size")}
    return None


def parse(data: dict) -> list[dict]:
    """MediaWiki query result -> Discover entries. Pure function (tests use a saved response)."""
    out = []
    pages = sorted(data.get("query", {}).get("pages", []), key=lambda p: p.get("index", 0))
    for p in pages:
        vi = (p.get("videoinfo") or [None])[0]
        if not vi:
            continue
        meta = vi.get("extmetadata") or {}

        def m(k):
            return _text((meta.get(k) or {}).get("value"))
        lic = m("LicenseShortName")
        w, h = int(vi.get("width") or 0), int(vi.get("height") or 0)
        if not licence_ok(lic) or w < MIN_WIDTH or not h or w / h < MIN_RATIO:
            continue
        if float(vi.get("duration") or 0) < MIN_SECONDS or m("Restrictions") or m("NonFree"):
            continue
        src = best_source(vi)
        if not src:
            continue
        title = m("ObjectName") or re.sub(r"\.\w+$", "", p["title"].removeprefix("File:"))
        out.append({"id": f"commons-{p['pageid']}", "pageid": p["pageid"], "title": title, "preview": vi.get("thumburl", ""),
                    "author": m("Artist") or vi.get("user", ""), "license": lic, "license_url": m("LicenseUrl"),
                    "url": vi.get("descriptionurl", ""), "download": src, "size": src.get("size") or 0,
                    "duration": float(vi.get("duration") or 0), "width": src["width"], "height": src["height"],
                    "desc": m("ImageDescription")[:300], "type": "video", "tags": []})
    return out


def browse(query: str = "", topic: str = "Nature", page: int = 1) -> dict:
    cat = TOPICS.get(topic, TOPICS["Nature"])
    search = f'filetype:video deepcat:"{cat}"' + (f" {query}" if query else "")
    data = _get({"action": "query", "generator": "search", "gsrsearch": search, "gsrnamespace": "6",
                 "gsrlimit": str(PAGE_SIZE), "gsroffset": str((max(page, 1) - 1) * PAGE_SIZE),
                 "prop": "videoinfo", "viprop": "url|size|mime|derivatives|extmetadata|user",
                 "viurlwidth": "480"})
    return {"items": parse(data), "has_next": "continue" in data, "page": page}


def attribution(r: dict) -> str:
    """One-line credit, as CC BY / BY-SA ask for it."""
    return f"\"{r['title']}\" by {r['author'] or 'unknown'}, {r['license']}, via Wikimedia Commons ({r['url']})"


def installed(pageid) -> Path | None:
    """The library folder of an already imported Commons file."""
    try:
        for d in paths.local_dir().iterdir():
            pj = d / "project.json"
            if pj.is_file() and json.loads(pj.read_text()).get("commons_pageid") == int(pageid):
                return d
    except Exception:
        pass
    return None


def download(r: dict, progress=None) -> Path:
    """Fetch the chosen file and import it as a library video with its licence and author recorded."""
    have = installed(r["pageid"])
    if have:
        return have
    src = r["download"]
    tmp = Path(tempfile.mkdtemp(prefix="wallforge-commons-"))
    try:
        dest = tmp / (importer._slug(r["title"]) + src["ext"])
        req = urllib.request.Request(src["url"], headers={"User-Agent": UA})
        got = 0
        with urllib.request.urlopen(req, timeout=60) as resp, open(dest, "wb") as f:
            total = int(resp.headers.get("Content-Length") or 0)
            if total > MAX_DOWNLOAD:
                raise RuntimeError("File is larger than 500 MB")
            while chunk := resp.read(1 << 20):
                got += len(chunk)
                if got > MAX_DOWNLOAD:
                    raise RuntimeError("File is larger than 500 MB")
                f.write(chunk)
                if progress and total:
                    progress(got / total)
        extra = {"source": SOURCE, "author": r["author"], "license": r["license"], "license_url": r["license_url"],
                 "source_url": r["url"], "attribution": attribution(r), "commons_pageid": int(r["pageid"]),
                 "description": r["desc"]}
        return importer.import_video(dest, copy=True, title=r["title"], extra=extra)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main(argv: list[str]) -> int:
    """wallforge --commons [QUERY] [--topic NAME] [--get N]: search from the terminal, optionally import result N."""
    topic, get, words = "Nature", None, []
    it = iter(argv)
    for a in it:
        if a == "--topic":
            topic = next(it, topic)
        elif a == "--get":
            get = int(next(it, "0"))
        else:
            words.append(a)
    if topic not in TOPICS:
        print(f"Unknown topic {topic!r}. Topics: {', '.join(TOPICS)}", file=sys.stderr)
        return 2
    res = browse(" ".join(words), topic)
    for i, r in enumerate(res["items"], 1):
        print(f"{i:2}. {r['title'][:60]} | {r['author'][:30]} | {r['license']} | {r['width']}x{r['height']} {r['duration']:.0f}s")
    if get:
        print(download(res["items"][get - 1]))
    return 0
