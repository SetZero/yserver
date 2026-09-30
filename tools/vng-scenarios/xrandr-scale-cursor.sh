# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #185: root GetImage under `xrandr --scale` (a SW cursor on yserver)
# must not contain the pointer. Captures the root with the pointer at two
# spots over a noise-tiled root; on both servers the two must be identical.
# shellcheck shell=sh
# golden: root-pixels.txt root.txt
set -u
set +e
xprop -root -spy > /dev/null 2>&1 &
hold=$!
sleep 1
out=$(xrandr | awk '/ connected/{print $1; exit}')
xrandr --output "$out" --scale 2x2 --filter nearest > xrandr.log 2>&1 || echo scale > FAILED
sleep 1
python3 - > tile.log 2>&1 <<'PY'
import random
from Xlib import X, display
d = display.Display()
s = d.screen()
root = s.root
random.seed(185)
tile = root.create_pixmap(64, 64, s.root_depth)
gc = tile.create_gc()
tile.put_image(gc, 0, 0, 64, 64, X.ZPixmap, s.root_depth, 0,
               bytes(random.randrange(256) if i % 4 != 3 else 0 for i in range(64 * 64 * 4)))
root.change_attributes(background_pixmap=tile)
root.clear_area(0, 0, 0, 0)
d.sync()
PY
sleep 1
for at in "301 203" "900 500"; do
    # shellcheck disable=SC2086
    xdotool mousemove $at
    sleep 1
    import -window root "root-${at% *}.png" > import.log 2>&1 || true
done
xwininfo -root | grep -E 'Width|Height' > root.txt
kill $hold 2>/dev/null || true
python3 - root-301.png root-900.png > root-pixels.txt 2>&1 <<'PY'
import hashlib, sys
from PIL import Image
for f in sys.argv[1:]:
    im = Image.open(f).convert("RGB")
    print(f, im.size, hashlib.sha256(im.tobytes()).hexdigest()[:16])
PY
if [ -e FAILED ]; then echo "fail: xrandr --scale 2x2 failed" > RESULT
elif [ "$(awk '{print $NF}' root-pixels.txt | sort -u | wc -l)" != 1 ]; then
    echo "fail: the root captures differ with the pointer moved (root-pixels.txt)" > RESULT
else echo pass > RESULT; fi
