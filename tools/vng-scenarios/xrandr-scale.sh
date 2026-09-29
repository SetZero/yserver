# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #185: what `xrandr --scale` does to screen size, CRTC info, monitors
# and the transform readback, per scale factor.
#   tools/vng-shot.sh --server xorg --dump none --name scale-xorg --scenario tools/vng-scenarios/xrandr-scale.sh
set -u
# Hold a connection: Xorg resets (dropping the transform) when its last client leaves.
xprop -root -spy > /dev/null 2>&1 &
hold=$!
sleep 1
out=$(xrandr | awk '/ connected/{print $1; exit}')
for s in 1.6 0.625 0.8 2 0.5 1.333333; do
    echo "===== --scale ${s}x${s}"
    xrandr --output "$out" --scale "${s}x${s}" 2>&1
    xrandr --verbose | grep -E '^Screen|^'"$out"'|Transform|^ {12}[-0-9]|filter:' 2>&1
    xrandr --listmonitors 2>&1
    xwininfo -root | grep -E 'Width|Height'
done > scale.log 2>&1
xrandr --output "$out" --scale 1x1 > /dev/null 2>&1 || true
kill $hold 2>/dev/null || true
