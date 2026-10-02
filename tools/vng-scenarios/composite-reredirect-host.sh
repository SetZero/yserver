#!/usr/bin/env bash
# Host half of composite-reredirect.sh: dumps the scanout once the probe has
# composited the re-redirected window's named pixmap into the COW, and checks
# it shows only that window's colour (the pointer, at 5,5, is masked).
#   tools/vng-scenarios/composite-reredirect-host.sh [name] [vng-shot args...]
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
name=${1:-composite-reredirect}
shift || true
out=${VNG_OUT:-$repo/target/vng}/$name
rm -rf "$out"
"$repo/tools/vng-shot.sh" --dump none --name "$name" --settle 0 --timeout 400 "$@" \
    --scenario "$repo/tools/vng-scenarios/composite-reredirect.sh" > /dev/null &
shot=$!
for _ in $(seq 1 600); do
    [ -e "$out/READY-0" ] && break
    kill -0 "$shot" 2>/dev/null || break
    sleep 0.5
done
if [ -e "$out/READY-0" ]; then
    "$repo/tools/qemu-monitor.py" "$out/monitor.sock" "sendkey ctrl-alt-ret"
    for _ in $(seq 1 40); do
        compgen -G "$out/yserver-scanout-*.ppm" > /dev/null && break
        sleep 0.25
    done
    sleep 0.5
    touch "$out/DONE-0"
fi
wait "$shot"
python3 - "$out" <<'PY'
import glob, sys
from PIL import Image
out = sys.argv[1]
files = sorted(glob.glob(f"{out}/yserver-scanout-*-out0-*.ppm"))
if not files:
    print("no scanout dump")
    sys.exit(1)
c = int(open(f"{out}/expect-0").read(), 16)
want = (c >> 16, (c >> 8) & 255, c & 255)
img = Image.open(files[0]).convert("RGB")
p = img.load()
w, h = img.size
bad = sum(p[x, y] != want for y in range(h) for x in range(w) if not (x < 64 and y < 64))
print(f"expect {c:06x}: {bad} wrong pixels ({files[0].rsplit('/', 1)[1]})")
sys.exit(1 if bad else 0)
PY
