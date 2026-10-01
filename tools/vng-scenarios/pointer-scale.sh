# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #185 Q6: does relative device motion (QEMU PS/2 mouse, via
# pointer-scale-host.sh) move the root pointer the same distance under a CRTC
# scale transform as without one?
# shellcheck shell=sh
# golden: pointer.log
# golden-include: masks/randr.txt
# mask: \bwindow:\d+ => window:<xid> -- window ids are server-assigned
set -u
set +e
if ! command -v xinput > /dev/null; then
    echo "fail: xinput missing, cannot make pointer acceleration flat" > RESULT
    return 0
fi
xprop -root -spy > /dev/null 2>&1 &
hold=$!
sleep 1
out=$(xrandr | awk '/ connected/{print $1; exit}')
for id in $(xinput list --id-only); do
    xinput set-prop "$id" 'libinput Accel Profile Enabled' 0 1 2>/dev/null || true
done
echo "accel: flat" >> pointer.log
phase() {
    xdotool mousemove 200 200
    sleep 0.5
    echo "=== $1 start $(xdotool getmouselocation)" >> pointer.log
    touch "READY-$1"
    for _ in $(seq 1 120); do [ -e "DONE-$1" ] && break; sleep 0.5; done
    [ -e "DONE-$1" ] || echo "$1" >> FAILED
    sleep 0.5
    echo "=== $1 end   $(xdotool getmouselocation)" >> pointer.log
}
phase identity
xrandr --output "$out" --scale 2x2
xrandr | grep -E '^Screen' >> pointer.log
phase scale2
xrandr --output "$out" --scale 1x1 || true
kill $hold 2>/dev/null || true
if [ -e FAILED ]; then echo "fail: host never moved the mouse in: $(paste -sd' ' FAILED)" > RESULT
else echo pass > RESULT; fi
