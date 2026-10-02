# Sourced by tools/vng-shot.sh INSIDE the guest, with DISPLAY=:7 exported and
# the artifact directory as cwd. Issue #150 — does XkbGetMap()/
# XkbGetUpdatedMap() leave xkb->server->behaviors NULL, the way barrierc
# needs it not to be (see tools/xkb-behaviors-probe.c for the full story)?
#
# Deliberately runs NO window manager, no xterm, nothing else that could
# touch the keymap: this is a pure protocol probe and anything else running
# is a confound, not cover. `--dump none` is right — no rendering is
# involved, and the load-bearing artifact is probe.log plus probe.exit.
#
#   tools/vng-shot.sh --dump none --name xkb-behaviors-ys \
#       --scenario tools/vng-scenarios/xkb-behaviors-probe.sh
#   tools/vng-shot.sh --server xorg --dump none --name xkb-behaviors-xorg \
#       --scenario tools/vng-scenarios/xkb-behaviors-probe.sh
# NOTE: this file is sourced into vng-shot.sh's guest.sh, which runs under
# `set -eu`. The probe's whole point is to exit nonzero (2) when behaviors[]
# would segfault, so its invocation MUST be shielded from -e (a plain failing
# statement here would abort the guest script before READY is ever touched,
# which is exactly what happened during development of this scenario).
set -u
src=$(dirname "$0")/../xkb-behaviors-probe.c
[ -r "$src" ] || src=${YSERVER_REPO:?}/tools/xkb-behaviors-probe.c

# A prebuilt binary is honoured if one was staged into the guest, but the
# checked-in artifact is the .c, so build it here the way every other scenario
# does rather than depending on a tools/xkb-behaviors-probe that no checkout
# ever contains.
bin=/tmp/xkb-behaviors-probe
if [ ! -x "$bin" ]; then
    cc -O1 -o xkb-behaviors-probe "$src" -lX11 -lxcb -lxcb-xkb > cc.log 2>&1 || {
        echo "xkb-behaviors: compile failed" >&2
        cat cc.log >&2
    }
    bin=./xkb-behaviors-probe
fi

if [ -x "$bin" ]; then
    "$bin" "$DISPLAY" > probe.log 2>&1 && rc=0 || rc=$?
    echo "$rc" > probe.exit
else
    echo "xkb-behaviors-probe: no runnable probe binary found" > probe.log
    echo 1 > probe.exit
fi

echo "--- probe.log ---"
cat probe.log
echo "--- probe.exit: $(cat probe.exit) ---"
