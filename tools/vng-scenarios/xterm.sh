# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# shellcheck shell=sh
# An xterm that maps; vng-shot then requires a scanout dump of it.
set -u
set +e
xterm -geometry 60x20+80+60 > xterm.log 2>&1 &
if timeout 30 xdotool search --sync --onlyvisible --class xterm > /dev/null 2>&1; then
    echo pass > RESULT
else
    echo "fail: xterm window never mapped" > RESULT
fi
