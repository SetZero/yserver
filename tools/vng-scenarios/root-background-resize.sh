# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# A tiled root background pixmap across screen resizes (scale, rotation):
# the root must keep showing the tile without any client clearing it.
# shellcheck shell=sh
# golden: root-pixels.txt
set -u
set +e
python3 - > bg.log 2>&1 <<'PY' &
import time
from Xlib import display, X
d = display.Display()
s = d.screen()
root = s.root
pm = root.create_pixmap(64, 64, s.root_depth)
rows = bytes(b for y in range(64) for x in range(64) for b in (x * 4, y * 4, (x ^ y) * 4, 0))
pm.put_image(pm.create_gc(), 0, 0, 64, 64, X.ZPixmap, s.root_depth, 0, rows)
root.change_attributes(background_pixmap=pm)
root.clear_area(0, 0, 0, 0)
d.sync()
open("BG-SET", "w").close()
# Keep the pixmap's owner (and the server, for Xorg) alive.
time.sleep(120)
PY
setter=$!
for _ in $(seq 1 50); do [ -e BG-SET ] && break; sleep 0.2; done
out=$(xrandr | awk '/ connected/{print $1; exit}')
xdotool mousemove 1 1
capture() {
    sleep 1
    import -window root "root-$1.png" > /dev/null 2>&1 || echo "import-$1" >> FAILED
}
capture initial
for step in "scale-0.8 --scale 0.8x0.8" "scale-1 --scale 1x1" "left --rotate left" "normal --rotate normal"; do
    tag=${step%% *}
    # shellcheck disable=SC2086
    xrandr --output "$out" ${step#* } > /dev/null 2>&1 || echo "xrandr-$tag" >> FAILED
    capture "$tag"
done
kill $setter 2>/dev/null || true
python3 - root-*.png > root-pixels.txt 2>&1 <<'PY'
import hashlib, sys
from PIL import Image
for f in sorted(sys.argv[1:]):
    im = Image.open(f).convert("RGB")
    print(f, im.size, hashlib.sha256(im.tobytes()).hexdigest()[:16])
PY
if [ ! -e BG-SET ]; then echo "fail: the background client never set the root (bg.log)" > RESULT
elif [ -e FAILED ]; then echo "fail: $(paste -sd' ' FAILED)" > RESULT
else echo pass > RESULT; fi
