# Sourced by tools/vng-shot.sh INSIDE the guest, with DISPLAY=:7 exported and
# the artifact directory as cwd. Which Expose events does a resize produce, for
# each combination of bit gravity and background? See tools/resize-expose-probe.c.
# Runs unchanged on yserver and on `--server xorg`, which is the whole point —
# the two answers are the deliverable.
#
# No window manager: the client's own XResizeWindow/XMoveWindow is the only
# configure it ever sees, so the geometry is exactly what was asked for. Every
# window is override-redirect for the same reason.
#
# Pure protocol — nothing is read back off the scanout — so `--dump none` is
# right and the load-bearing artifacts are probe.log plus probe.exit.
#
#   tools/vng-shot.sh --dump none --name resize-expose-ys \
#       --scenario tools/vng-scenarios/resize-expose.sh
#   tools/vng-shot.sh --server xorg --dump none --name resize-expose-xorg \
#       --scenario tools/vng-scenarios/resize-expose.sh
#
# NOTE: this file is sourced into vng-shot.sh's guest.sh, which runs under
# `set -eu`. The probe's whole point is to exit 2 when a server disagrees with
# the Xorg-derived expectations, and it does exit 2 on yserver today, so its
# invocation MUST be shielded from -e — a plain failing statement here would
# abort the guest script before READY is ever touched.
#
# There is deliberately no `xwininfo -root -tree` step: the probe takes each
# window through its phases and then destroys it, so by the time the scenario
# could look there is nothing left to see and tree.txt came back empty. The
# probe's own stdout is the artifact.
set -u
src=$(dirname "$0")/../resize-expose-probe.c
[ -r "$src" ] || src=${YSERVER_REPO:?}/tools/resize-expose-probe.c

cc -O1 -o resize-expose-probe "$src" -lX11 > cc.log 2>&1 || {
    echo "resize-expose: compile failed" >&2
    cat cc.log >&2
}
if [ -x ./resize-expose-probe ]; then
    ./resize-expose-probe "$DISPLAY" > probe.log 2>&1 && rc=0 || rc=$?
    echo "$rc" > probe.exit
else
    echo "resize-expose-probe: no runnable probe binary found" > probe.log
    echo 1 > probe.exit
fi

echo "--- probe.log ---"
cat probe.log
echo "--- probe.exit: $(cat probe.exit) ---"
