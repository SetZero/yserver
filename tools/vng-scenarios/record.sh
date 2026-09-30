# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #180: every tools/record-probe.c scenario, logged to record.log, then
# a recorder listening for physical keys (physical.log) while
# record-host.sh presses them through the QEMU monitor:
#   tools/vng-scenarios/record-host.sh [name] [vng-shot args...]
# shellcheck shell=sh
# golden: record.log physical.log
# mask: \bclient=0x[0-9a-f]+ => client=<xid> -- the holding xprop's client id base is server-assigned
# mask: (setup success=1 major=11 len=)\d+ => \1<len> -- setup reply length follows each server's visual and format lists
# mask: (BadValue value=)(ctl\+1|0)( minor=1 major=RECORD) => \1<stale>\3 -- known (docs/status.md RECORD entry): stale Xorg error values are sent as 0
# drop: ^rec: several repeated presses$ -- known (docs/status.md RECORD entry): XTEST-held keys do not autorepeat on yserver
set -u
set +e
cc -O1 -o probe ${YSERVER_REPO:?}/tools/record-probe.c \
    -lxcb -lxcb-record -lxcb-xtest > cc.log 2>&1 || cat cc.log >&2
# Xorg 21.1 aborts (free(): invalid pointer) in the server reset after the
# `basic` scenario's clients leave: hold a connection so it never resets.
xprop -root -spy > /dev/null 2>&1 &
sleep 1
for s in "basic l 0" "basic l 7" "basic B 7" "basic B 0" "ranges l" "errors l" "errors B" \
    "free l" "recdie l" "ownerdie l" "repeat l"; do
    echo "=== $s"
    # shellcheck disable=SC2086
    timeout 20 ./probe $s 2>&1
done > record.log 2>&1
echo "=== end" >> record.log
(
    RECORD_PROBE_READY=LISTENING ./probe listen l 12 > physical.log 2>&1; rc=$?
    if [ ! -x probe ]; then echo "fail: probe did not build (cc.log)"
    elif [ "$rc" -ne 0 ]; then echo "fail: the physical-key recorder exited $rc (physical.log)"
    else echo pass; fi > RESULT
) &
