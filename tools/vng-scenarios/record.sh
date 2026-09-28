# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #180: every tools/record-probe.c scenario, logged to record.log, then
# a recorder listening for physical keys (physical.log) while
# record-host.sh presses them through the QEMU monitor:
#   tools/vng-scenarios/record-host.sh yserver|xorg
#   diff target/vng/record-xorg/physical.log target/vng/record-yserver/physical.log
set -u
cc -O1 -o probe /home/jos/Projects/yserver/tools/record-probe.c \
    -lxcb -lxcb-record -lxcb-xtest > cc.log 2>&1 || cat cc.log >&2
# Xorg 21.1 aborts (free(): invalid pointer) in the server reset after the
# `basic` scenario's clients leave, Xvfb too unless run with -noreset, so
# record-host.sh runs only the physical phase there: diff record.log against
# a host `Xvfb -noreset` run instead.
[ -n "${RECORD_PHYSICAL_ONLY:-}" ] || for s in "basic l 0" "basic l 7" "basic B 7" "basic B 0" "ranges l" "errors l" "errors B" \
    "free l" "recdie l" "ownerdie l" "repeat l"; do
    echo "=== $s"
    timeout 20 ./probe $s 2>&1
done > record.log 2>&1
RECORD_PROBE_READY=LISTENING ./probe listen l 12 > physical.log 2>&1 &
