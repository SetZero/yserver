# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Independent pixel oracle: a generated root pattern and a window of known
# contents, through layout changes (scale, rotation, a second output), bursts
# of partial window updates and cursor sweeps across every output. Each
# snapshot writes snap-<n>.json (the requested layout, what RANDR reports,
# the window's fills, the cursor path) and waits for pixel-oracle-host.sh to
# dump and check every output against the pattern pushed through its CRTC.
# shellcheck shell=sh
set -u
set +e
python3 - > oracle.log 2>&1 <<'PY'
import json, os, random, subprocess, time
from Xlib import display, X
from Xlib.ext import randr

d = display.Display()
s = d.screen()
root = s.root
rng = random.Random(1930)

def tri(t):
    t &= 511
    return t if t < 256 else 511 - t

# Root pattern, a 512x512 tile: r = tri(x), g = tri(y), b = tri(x + y).
T = 512
tile = root.create_pixmap(T, T, s.root_depth)
tgc = tile.create_gc()
for y0 in range(0, T, 32):
    rows = bytearray()
    for y in range(y0, y0 + 32):
        for x in range(T):
            rows += bytes((tri(x + y), tri(y), tri(x), 0))
    tile.put_image(tgc, 0, y0, T, 32, X.ZPixmap, s.root_depth, 0, bytes(rows))
root.change_attributes(background_pixmap=tile)
root.clear_area(0, 0, 0, 0)

# A solid 16x16 cursor, hotspot 0,0.
src = root.create_pixmap(16, 16, 1)
bgc = src.create_gc(foreground=1)
src.fill_rectangle(bgc, 0, 0, 16, 16)
cursor = src.create_cursor(src, (65535, 0, 65535), (0, 65535, 0), 0, 0)
root.change_attributes(cursor=cursor)

WX, WY, WW, WH = 500, 300, 900, 350
win = root.create_window(WX, WY, WW, WH, 0, s.root_depth, X.InputOutput, X.CopyFromParent,
                         background_pixel=0x202020, override_redirect=True,
                         event_mask=X.ExposureMask)
win.map()
wgc = win.create_gc()
fills = []

def fill(x, y, w, h, c):
    wgc.change(foreground=c)
    win.fill_rectangle(wgc, x, y, w, h)
    fills.append([x, y, w, h, c])

def repaint_on_expose():
    exposed = False
    while d.pending_events():
        exposed |= d.next_event().type == X.Expose
    if exposed:
        print("expose: repainting", len(fills), "fills", flush=True)
        for x, y, w, h, c in list(fills):
            wgc.change(foreground=c)
            win.fill_rectangle(wgc, x, y, w, h)
        d.sync()
    return exposed

fill(0, 0, WW, WH, 0x405060)
d.sync()

M1, M2 = (1280, 800), (1024, 768)
def out(name, mode, pos, rotate="normal", scale=1, filt="nearest"):
    return dict(name=name, mode=mode, pos=pos, rotate=rotate, scale=scale, filter=filt)
def v1(**k): return out("Virtual-1", M1, (0, 0), **k)
def v2(x, **k): return out("Virtual-2", M2, (x, 0), **k)
steps = [
    ("single", [v1()]),
    ("scale-1.6-bilinear", [v1(scale=1.6, filt="bilinear")]),
    ("scale-0.8", [v1(scale=0.8)]),
    ("left", [v1(rotate="left")]),
    ("right", [v1(rotate="right")]),
    ("inverted", [v1(rotate="inverted")]),
    ("dual", [v1(), v2(1280)]),
    ("dual-scale-0.8", [v1(scale=0.8), v2(1024)]),
    ("dual-left", [v1(), v2(1280, rotate="left")]),
    ("dual-inverted-scale-1.6-bilinear", [v1(rotate="inverted"), v2(1280, scale=1.6, filt="bilinear")]),
    ("dual-back", [v1(), v2(1280)]),
]

def xrandr(outs):
    args = ["xrandr"]
    names = {o["name"] for o in outs}
    for o in outs:
        # xrandr's own XDoubleToFixed truncation: the host oracle uses the same.
        sc = o["scale"]
        args += ["--output", o["name"], "--mode", "%dx%d" % tuple(o["mode"]),
                 "--pos", "%dx%d" % tuple(o["pos"]), "--rotate", o["rotate"],
                 "--scale", f"{sc}x{sc}", "--filter", o["filter"]]
    for n in ("Virtual-1", "Virtual-2"):
        if n not in names:
            args += ["--output", n, "--off"]
    r = subprocess.run(args, capture_output=True, text=True)
    print(" ".join(args), "rc", r.returncode, r.stderr.strip(), flush=True)
    return r.returncode

def reported():
    res = randr.get_screen_resources(root)
    modes = {m.id: (m.width, m.height) for m in res.modes}
    rep = []
    for o in res.outputs:
        oi = randr.get_output_info(d, o, res.config_timestamp)
        if not oi.crtc:
            continue
        ci = randr.get_crtc_info(d, oi.crtc, res.config_timestamp)
        t = randr.get_crtc_transform(d, oi.crtc)
        rep.append(dict(name=oi.name, pos=[ci.x, ci.y], size=[ci.width, ci.height],
                        mode=list(modes.get(ci.mode, (0, 0))), rotation=ci.rotation,
                        transform=[t.current_transform[f'matrix{i}{j}'] for i in (1, 2, 3) for j in (1, 2, 3)], filter=t.current_filter_name))
    g = root.get_geometry()
    return rep, [g.width, g.height]

snap = 0
for tag, outs in steps:
    rc = xrandr(outs)
    time.sleep(1.5)
    rep, screen = reported()
    # Vertical boundaries between outputs, in root coordinates.
    edges = sorted({o["pos"][0] for o in outs if o["pos"][0] > 0})
    for rnd in range(3 if len(outs) > 1 else 1):
        path = []
        for _ in range(12):
            p = (rng.randrange(screen[0]), rng.randrange(screen[1]))
            root.warp_pointer(*p)
            d.sync()
            path.append(p)
            time.sleep(0.04)
        for i in range(40):
            if edges and (i % 3 == 0 or i >= 35):
                ex = edges[0] - WX
                x = rng.randrange(max(0, ex - 200), ex)
                w = rng.randrange(ex - x + 1, ex - x + 200)
            else:
                x = rng.randrange(WW - 20)
                w = rng.randrange(1, WW - x)
            y = rng.randrange(WH - 10)
            h = rng.randrange(1, WH - y)
            fill(x, y, w, h, rng.randrange(1 << 24))
            d.sync()
            time.sleep(rng.random() * 0.03)
        final = (rng.randrange(screen[0]), rng.randrange(screen[1]))
        root.warp_pointer(*final)
        d.sync()
        time.sleep(1)
        while repaint_on_expose():
            time.sleep(0.5)
        q = root.query_pointer()
        with open(f"snap-{snap}.json", "w") as f:
            json.dump(dict(tag=f"{tag}.{rnd}", xrandr_rc=rc, requested=outs, reported=rep,
                           screen=screen, window=[WX, WY, WW, WH], fills=fills,
                           cursor=[q.root_x, q.root_y], cursor_path=path), f)
        open(f"READY-{snap}", "w").close()
        while not os.path.exists(f"DONE-{snap}"):
            time.sleep(0.2)
        snap += 1
open("STEPS-DONE", "w").write(f"{snap}\n")
PY
if [ -e STEPS-DONE ]; then echo pass > RESULT; else echo "fail: oracle client stopped early (oracle.log)" > RESULT; fi
