#!/usr/bin/env python3
"""Headless batch: render every scene wallpaper to a PNG and classify the result.
usage: python3 -I batch.py [--bin PATH] [--out DIR] [--size WxH] [--time S] [ids...]
Writes DIR/<id>.png, DIR/<id>.log, DIR/results.json and prints a table."""
import json, os, re, subprocess, sys, glob
a = sys.argv[1:]
def opt(n, d):
    if n in a:
        i = a.index(n); v = a[i+1]; del a[i:i+2]; return v
    return d
here = os.path.dirname(os.path.abspath(__file__))
BIN = opt('--bin', os.path.join(here, '..', 'target', 'release', 'wallforge-render'))
OUT = opt('--out', '/tmp/oc/shots'); SIZE = opt('--size', '640x360'); TIME = opt('--time', '2')
W = os.path.expanduser('~/.local/share/Steam/steamapps/workshop/content/431960')
os.makedirs(OUT, exist_ok=True)
ids = a or sorted(os.listdir(W))
res = {}
for i in ids:
    d = os.path.join(W, i)
    try: pj = json.load(open(os.path.join(d, 'project.json'), encoding='utf-8-sig'))
    except Exception: continue
    if str(pj.get('type', '')).lower() != 'scene': continue
    png = os.path.join(OUT, i + '.png')
    if os.path.exists(png): os.remove(png)
    try:
        p = subprocess.run([BIN, '--bg', d, '--silent', '--screenshot', png, '--screenshot-size', SIZE, '--screenshot-time', TIME],
                           capture_output=True, text=True, timeout=10)
        code, err = p.returncode, p.stderr
    except subprocess.TimeoutExpired as e:
        code, err = 124, (e.stderr.decode() if e.stderr else '') + 'timeout'
    open(os.path.join(OUT, i + '.log'), 'w').write(err)
    lines = [l for l in err.splitlines() if l.startswith('wallforge-render:')]
    skipped = [l for l in lines if 'skipped' in l or 'unavailable' in l or 'not found' in l]
    shader_err = [l for l in lines if 'shader' in l and ('fragment:' in l or 'vertex:' in l or 'link:' in l)]
    if code == 3: cls = 'unsupported'
    elif code != 0 or not os.path.exists(png): cls = 'fails'
    elif skipped: cls = 'partial'
    else: cls = 'works'
    res[i] = {'title': pj.get('title'), 'class': cls, 'code': code, 'skipped': skipped, 'shader_err': shader_err,
              'one_liner': (lines[-1] if lines else err.strip()[-160:])[:200]}
json.dump(res, open(os.path.join(OUT, 'results.json'), 'w'), indent=1)
cnt = {}
for r in res.values(): cnt[r['class']] = cnt.get(r['class'], 0) + 1
for i, r in res.items(): print(f"{i:>12} {r['class']:<11} exit={r['code']:<3} {r['title'][:28]:<28} {r['one_liner'][:90]}")
print(cnt)
