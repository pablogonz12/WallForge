#!/usr/bin/env python3
"""Contact sheets of batch.py renders next to each wallpaper's own preview image, to judge them by eye.
usage: python3 -I compare.py SHOTS_DIR [--before OLD_SHOTS_DIR] [--out DIR] [--per N] [--workshop DIR] [ids...]
Each tile is: [render before |] render | preview, labelled with the Workshop id and title. Needs ImageMagick."""
import json, os, subprocess, sys
a = sys.argv[1:]
def opt(n, d):
    if n in a:
        i = a.index(n); v = a[i+1]; del a[i:i+2]; return v
    return d
before = opt('--before', ''); per = int(opt('--per', '16'))
W = os.path.expanduser(opt('--workshop', '~/.local/share/Steam/steamapps/workshop/content/431960'))
out = opt('--out', '')
if not a:
    sys.exit(__doc__)
shots = a.pop(0); out = out or os.path.join(shots, 'sheets')
ids = a or sorted(f[:-4] for f in os.listdir(shots) if f.endswith('.png'))
os.makedirs(out, exist_ok=True)
tiles = []
for i in ids:
    try: pj = json.load(open(os.path.join(W, i, 'project.json'), encoding='utf-8-sig'))
    except Exception: pj = {}
    pv = os.path.join(W, i, pj.get('preview') or 'preview.jpg')
    cols = ([os.path.join(before, i + '.png')] if before else []) + [os.path.join(shots, i + '.png'), pv]
    cmd = ['magick']
    for c in cols:
        cmd += ['(', (c + '[0]') if os.path.exists(c) else 'xc:red', '-resize', '280x158!', ')']
    t = os.path.join(out, 't_%s.jpg' % i)
    cmd += ['+append', '-gravity', 'south', '-background', 'black', '-fill', 'white', '-pointsize', '14', '-splice', '0x18',
            '-annotate', '+0+1', '%s %s' % (i, str(pj.get('title', ''))[:40]), t]
    subprocess.run(cmd, capture_output=True); tiles.append(t)
n = 0
for n, k in enumerate(range(0, len(tiles), per)):
    subprocess.run(['montage', *tiles[k:k+per], '-tile', '2x', '-geometry', '+3+3', '-background', '#222', os.path.join(out, 'sheet%d.jpg' % n)])
for t in tiles: os.remove(t)
print('%d sheet(s) in %s (a red tile means the image is missing)' % (n + 1 if tiles else 0, out))
