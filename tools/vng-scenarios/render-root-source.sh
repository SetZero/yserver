# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
#
# Issue #135, reduced: NO window manager and NO Composite redirect, just the
# one thing under test — a RENDER Composite sourcing a Picture on the root with
# subwindow-mode=IncludeInferiors — with a plain XGetImage(root) alongside it
# as an in-run control. Runs unchanged on yserver and on Xorg
# (`--server xorg`), so Xorg supplies the expected pixels.
set -u
src=$(dirname "$0")/render-root-source-client.c
[ -r "$src" ] || src=${YSERVER_REPO:?}/tools/vng-scenarios/render-root-source-client.c
cc -O1 -o render-root-source-client "$src" -lX11 -lXrender > cc.log 2>&1 || {
    echo "render-root-source: compile failed" >&2; cat cc.log >&2; }
if [ -x ./render-root-source-client ]; then
    ./render-root-source-client > client.log 2>&1 &
    sleep 6
fi
xwininfo -root -tree > tree.txt 2>&1 || true
