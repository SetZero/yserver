#!/usr/bin/env python3
"""Host check for pixel-oracle.sh: every output's scanout of every snapshot
against the generated root pattern plus the window's recorded fills, pushed
through that output's CRTC transform. Exit 1 on any wrong pixel.

  pixel-oracle-check.py <artifact-dir>

Scanout pixel d of a W x H mode shows root pixel pos + S.R.(d + 1/2)
(RRTransformCompute, rrtransform.c:167-253): nearest takes floor(p - eps)
(pixman), bilinear blends the four pixel centres around p. Only the final
cursor's box is masked; the rectangles it vacated are checked like the rest.
"""
import glob
import json
import math
import sys

from PIL import Image, ImageChops, ImageDraw

BILINEAR_TOL = 4
TIE = 1e-3
CURSOR_BOX = (-4, 40)


def tri(t):
    t &= 511
    return t if t < 256 else 511 - t


def pattern_tile():
    t = Image.new("RGB", (512, 512))
    t.putdata([(tri(x), tri(y), tri(x + y)) for y in range(512) for x in range(512)])
    return t


def expected_root(snap, tile):
    w, h = snap["screen"]
    img = Image.new("RGB", (w, h))
    for y in range(0, h, 512):
        for x in range(0, w, 512):
            img.paste(tile, (x, y))
    wx, wy, ww, wh = snap["window"]
    draw = ImageDraw.Draw(img)
    for x, y, fw, fh, c in snap["fills"]:
        x0, y0 = max(x, 0), max(y, 0)
        x1, y1 = min(x + fw, ww), min(y + fh, wh)
        if x1 > x0 and y1 > y0:
            draw.rectangle((wx + x0, wy + y0, wx + x1 - 1, wy + y1 - 1),
                           fill=(c >> 16, (c >> 8) & 255, c & 255))
    return img


def fixed(v):
    # xrandr's XDoubleToFixed truncates.
    return int(v * 65536) / 65536


def crtc_map(o):
    """Affine (a, b, c, d, e, f): root = (a x + b y + c, d x + e y + f)."""
    w, h = o["mode"]
    rot = {
        "normal": (1, 0, 0, 0, 1, 0),
        "left": (0, -1, h, 1, 0, 0),
        "inverted": (-1, 0, w, 0, -1, h),
        "right": (0, 1, 0, -1, 0, w),
    }[o["rotate"]]
    s = fixed(o["scale"])
    px, py = o["pos"]
    a, b, c, d, e, f = rot
    return (s * a, s * b, s * c + px, s * d, s * e, s * f + py)


def sample(m, x, y):
    a, b, c, d, e, f = m
    cx, cy = x + 0.5, y + 0.5
    return a * cx + b * cy + c, d * cx + e * cy + f


def bilinear(rp, w, h, px, py):
    ux, uy = px - 0.5, py - 0.5
    x0, y0 = math.floor(ux), math.floor(uy)
    fx, fy = ux - x0, uy - y0
    acc = [0.0, 0.0, 0.0]
    for yy, wy in ((y0, 1 - fy), (y0 + 1, fy)):
        for xx, wx in ((x0, 1 - fx), (x0 + 1, fx)):
            v = rp[min(max(xx, 0), w - 1), min(max(yy, 0), h - 1)]
            for k in range(3):
                acc[k] += v[k] * wx * wy
    return acc


def check_output(snap, o, scan, root, tol):
    m = crtc_map(o)
    w, h = o["mode"]
    rw, rh = root.size
    exp = root.transform((w, h), Image.AFFINE,
                         (m[0], m[1], m[2] - 1e-6, m[3], m[4], m[5] - 1e-6),
                         resample=Image.BILINEAR if o["filter"] == "bilinear" else Image.NEAREST)
    diff = ImageChops.difference(exp, scan)
    bands = [b.point(lambda v: 255 if v > tol else 0) for b in diff.split()]
    mask = ImageChops.lighter(ImageChops.lighter(bands[0], bands[1]), bands[2])
    if mask.getbbox() is None:
        return 0, 0, 0, []
    cx, cy = snap["cursor"]
    boxes = [(x, y) for x, y in snap["cursor_path"]]
    sp, rp = scan.load(), root.load()
    bad = masked = vacated = 0
    first = []
    data = mask.tobytes()
    i = data.find(b"\xff")
    while i >= 0:
        x, y = i % w, i // w
        i = data.find(b"\xff", i + 1)
        px, py = sample(m, x, y)
        if cx + CURSOR_BOX[0] <= px < cx + CURSOR_BOX[1] and cy + CURSOR_BOX[0] <= py < cy + CURSOR_BOX[1]:
            masked += 1
            continue
        got = sp[x, y]
        if o["filter"] == "bilinear":
            if px < 1.5 or py < 1.5 or px > rw - 1.5 or py > rh - 1.5:
                continue
            want = bilinear(rp, rw, rh, px, py)
            if all(abs(g - v) <= tol for g, v in zip(got, want)):
                continue
        else:
            cands = {(math.floor(px + dx), math.floor(py + dy))
                     for dx in (-TIE, TIE) for dy in (-TIE, TIE)}
            if any(0 <= qx < rw and 0 <= qy < rh and rp[qx, qy] == got for qx, qy in cands):
                continue
            want = rp[min(max(math.floor(px - 1e-6), 0), rw - 1), min(max(math.floor(py - 1e-6), 0), rh - 1)]
        bad += 1
        if any(bx + CURSOR_BOX[0] <= px < bx + CURSOR_BOX[1] and by + CURSOR_BOX[0] <= py < by + CURSOR_BOX[1]
               for bx, by in boxes):
            vacated += 1
        if len(first) < 5:
            first.append(f"scan({x},{y}) root({px:.1f},{py:.1f}) got {got} want {tuple(round(v) for v in want)}")
    return bad, masked, vacated, first


def main():
    art = sys.argv[1]
    tile = pattern_tile()
    snaps = sorted(glob.glob(f"{art}/snap-*.json"), key=lambda p: int(p.rsplit("-", 1)[1][:-5]))
    if not snaps:
        print("no snapshots")
        return 1
    failed = 0
    for path in snaps:
        n = int(path.rsplit("-", 1)[1][:-5])
        snap = json.load(open(path))
        req = snap["requested"]
        problems = []
        if snap["xrandr_rc"] != 0:
            problems.append(f"xrandr exited {snap['xrandr_rc']}")
        rep = {r["name"]: r for r in snap["reported"]}
        for o in req:
            r = rep.get(o["name"])
            if r is None or r["pos"] != list(o["pos"]) or r["mode"] != list(o["mode"]):
                problems.append(f"{o['name']}: RANDR reports {r}")
        root = expected_root(snap, tile)
        line = []
        # yserver numbers its dumps by pool, not RANDR order: match by size.
        dumps = {}
        for f in glob.glob(f"{art}/scanout-{n}-out*"):
            if not f.endswith(".ppm"):
                problems.append(f"unreadable output: {open(f).read().strip()}")
                continue
            im = Image.open(f).convert("RGB")
            dumps.setdefault(im.size, []).append(im)
        if len(req) != sum(map(len, dumps.values())):
            problems.append(f"{sum(map(len, dumps.values()))} dumps for {len(req)} outputs")
        for o in req:
            got = dumps.get(tuple(o["mode"]), [])
            if len(got) != 1:
                problems.append(f"{o['name']}: {len(got)} dumps of size {o['mode']}")
                continue
            scan = got[0]
            tol = BILINEAR_TOL if o["filter"] == "bilinear" else 0
            bad, masked, vacated, first = check_output(snap, o, scan, root, tol)
            line.append(f"{o['name']} wrong {bad} (vacated cursor boxes {vacated}, cursor masked {masked})")
            if bad:
                problems.append(f"{o['name']}: {bad} wrong pixels: " + "; ".join(first))
        print(f"snap {n} {snap['tag']}: " + ", ".join(line))
        for p in problems:
            print(f"  FAIL {p}")
        failed += bool(problems)
    print(f"{failed} of {len(snaps)} snapshots wrong")
    return 1 if failed else 0


sys.exit(main())
