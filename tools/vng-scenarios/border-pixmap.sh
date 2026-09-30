# Sourced by tools/vng-shot.sh INSIDE the guest, with DISPLAY=:7 exported and
# the artifact directory as cwd. Tiled window borders (CWBorderPixmap) — the
# path the awesome smoke provably cannot reach (#133).
#
# Deliberately runs NO window manager: the client's windows are
# override-redirect, so nothing repositions or reparents them and every ring
# pixel has a predictable absolute coordinate. Works unchanged on yserver and
# on Xorg (`--server xorg`), which makes it a parity check as well.
#
# Check the capture with:
#     tools/border-pixmap-check.py <scanout.ppm>
set -u
src=$(dirname "$0")/border-pixmap-client.c
[ -r "$src" ] || src=${YSERVER_REPO:?}/tools/vng-scenarios/border-pixmap-client.c

cc -O1 -o border-pixmap-client "$src" -lX11 > cc.log 2>&1 || {
    echo "border-pixmap: compile failed" >&2
    cat cc.log >&2
}
if [ -x ./border-pixmap-client ]; then
    ./border-pixmap-client > client.log 2>&1 &
    sleep 3
fi

xwininfo -root -tree > tree.txt 2>&1 || true
import -window root screen.png 2>&1 | head -3 > import.log || true
