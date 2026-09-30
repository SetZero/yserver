# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #132: `xrandr --dpi` must reach NEW clients' setup reply (Xft's DPI).
# shellcheck shell=sh
# golden: dpi.log
# golden-include: masks/randr.txt
# mask: \(33[89]x21[12] millimeters\) => (<startup mm> millimeters) -- known (docs/status.md 2026-09-29): startup screen mm round to 339x212, Xorg truncates to 338x211
set -u
set +e
# Hold a connection: Xorg resets (reverting the mm) when its last client leaves.
xprop -root -spy > /dev/null 2>&1 &
hold=$!
sleep 1
{
    echo "=== before"; xdpyinfo | grep -E 'dimensions|resolution'
    xrandr --dpi 108 || echo "xrandr --dpi failed" > FAILED-DPI
    echo "=== after xrandr --dpi 108"; xdpyinfo | grep -E 'dimensions|resolution'
} > dpi.log 2>&1 || true
kill $hold 2>/dev/null || true
if [ -e FAILED-DPI ]; then echo "fail: xrandr --dpi 108 failed" > RESULT; else echo pass > RESULT; fi
