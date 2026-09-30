# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
# Issue #173: XI2 raw key events (XI_RawKeyPress / XI_RawKeyRelease). No WM,
# no windows: focus stays PointerRoot, so root selectors also see the device
# events.
#
# Two phases, both logged with the XI opcode, sequence numbers and timestamps
# blanked (xtest.log also drops the inter-event time deltas) so the Xorg and
# yserver runs diff directly:
#
# xtest.log     XTEST-driven cases: every selection form, duplicate presses,
#               a stray release, core/XI2 grabs (sync, async, passive,
#               owner_events, XI 2.0 vs 2.2 clients), XI version storage.
# physical.log  real keys on the guest's PS/2 keyboard, pressed by the host
#               through the QEMU monitor while the guest holds (a long press
#               covers auto-repeat). Needs tools/vng-scenarios/xi2-raw-keys-host.sh,
#               which runs vng-shot and presses the keys:
#
#   tools/vng-scenarios/xi2-raw-keys-host.sh [name] [vng-shot args...]
#
# Expected differences outside the raw events (2026-09-26 run): xtest.log —
# XIGrabDevice(keyboard) sends the grabber XI_FocusIn/Out it never selected,
# XI2 device events carry a different pointer position, and XIQueryVersion
# answers a changed request with the new version instead of the stored one /
# BadValue; physical.log — the physical keyboard's slave id (Xorg: its own
# device, 7 here; yserver: slave 5) and auto-repeat (Xorg: XI2 KeyPress with
# XIKeyRepeat and no release; yserver: release+press pairs). Raw events agree:
# none for repeats on either server.
# shellcheck shell=sh
# golden: xtest.log physical-by-client.log
# mask: \bdt=-?\d+ => dt=<t> -- inter-event time
# mask: ^(\S+   wire \S+ 16000000 0[23]000300 \S+ \S+) \S+ \S+ \S+ (\S+ \S+) \S+ \S+ => \1 <root> <event> <child> \2 <ex> <ey> -- window ids are server-assigned; event_x/y differ (known, see above)
# drop: ^\S+ \?\s+dev=3 src=0 detail=\d+ event=\w+ flags=0x0 len=11$ -- known (see above): XIGrabDevice(keyboard) sends XI_FocusIn/Out the grabber never selected
# drop: ^\S+   wire \S+ 0b000000 0[9a]000300 -- the same focus events' wire bytes
# mask: ^(v20then22 xi-version asked=2.2) got=2\.[02]$ => \1 got=<known> -- known (see above): XIQueryVersion answers a changed request with the new version
# mask: ^(v22then20 xi-version asked=2.0) (error=2|got=2\.0)$ => \1 <known> -- known (see above): the same XIQueryVersion divergence
# drop: core-event type=34$ -- MappingNotify: Xorg switches the master's classes between slaves, yserver has one slave
# mask: \b(dev|src)=[57]\b => \1=<kbd> -- known (see above): the physical keyboard is its own slave 7 on Xorg, slave 5 on yserver
# mask: ^(\S+   wire \S+ 02000000 0[de]000)[357](00 \S+ \S+) 0[57]000200 => \1<d>\2 <kbd>000200 -- the same slave ids in the raw events' wire bytes
# drop: ^\S+ Key(Press|Release)\s+dev=\S+ src=\S+ detail=56 -- known (see above): the held b auto-repeats as XIKeyRepeat presses on Xorg, release+press pairs on yserver
# drop: ^\S+   wire \S+ 16000000 0[23]000300 \S+ 38000000 -- the same repeats' wire bytes (detail 56)
# mask: (<ex> <ey> 08000200 )0[57]000000 => \1<kbd>000000 -- the slave id as sourceid in the device events' wire bytes
set -u
set +e
src=${YSERVER_REPO:?}/tools/vng-scenarios/xi2-raw-keys-probe.c
cc -O1 -o probe "$src" -lxcb -lxcb-xinput -lxcb-xtest > cc.log 2>&1 || cat cc.log >&2

./probe list > devices.log 2>&1 || true

M="mon:m22:2:1 mon:a22:2:0 mon:vck:2:3 mon:xtk:2:5 mon:m20:0:1 mon:a20:0:0"
run() {
    # MappingNotify (core event 34) is Xorg switching the master keyboard's
    # classes to the XTEST slave; yserver has one slave, so none. Not part of
    # the raw-event comparison.
    # shellcheck disable=SC2086
    ./probe $M "$@" 2>&1 | grep -v 'core-event type=34' | sed -E 's/ dt=-?[0-9]+//'
    echo "======"
}
{
    run '# basic' p38 r38
    run '# duplicate press, non-modifier' p38 p38 r38
    run '# duplicate press, modifier (Shift_L)' p50 p50 r50
    run '# release of a key that is not down' r38
    run '# G1 core GrabKeyboard async' grabkbd:async p38 r38 ungrabkbd p38 r38
    run '# C1 core GrabKeyboard, grabber selects raw on root' drvsel:1 grabkbd:async p38 r38 ungrabkbd
    run '# G2 XIGrabDevice(3) root, raw in grab mask, grabber selects' drvsel:1 xigrab:3 p38 r38 xiungrab:3
    run '# G3 XIGrabDevice(3) child, raw in grab mask, grabber selects' drvsel:1 xigrab:3:win p38 r38 xiungrab:3
    run '# G3b XIGrabDevice(3) root, raw in grab mask' xigrab:3 p38 r38 xiungrab:3
    run '# O1 XIGrabDevice(3) child owner_events, no raw in mask, grabber selects' drvsel:1 xigrab:3:win:oe:noraw p38 r38 xiungrab:3
    run '# O2 XIGrabDevice(3) child, no raw in mask, grabber selects' drvsel:1 xigrab:3:win:noraw p38 r38 xiungrab:3
    run '# O3 XIGrabDevice(3) root owner_events, no raw in mask, grabber selects' drvsel:1 xigrab:3:oe:noraw p38 r38 xiungrab:3
    run '# O4 XIGrabDevice(3) root, no raw in mask, grabber selects' drvsel:1 xigrab:3:noraw p38 r38 xiungrab:3
    run '# G4 core GrabKeyboard sync, AllowEvents(AsyncKeyboard)' grabkbd:sync p38 r38 allow:async p39 r39 ungrabkbd
    run '# G5 passive GrabKey sync, AllowEvents(ReplayKeyboard)' grabkey:38:sync p38 r38 allow:replay p39 r39
    run '# G6 passive GrabKey async' grabkey:38:async p38 r38 p39 r39
    ./probe mon:none:-:1 mon:v20then22:0,2:1 mon:v22then20:2,0:1 mon:v23then22:3,2:1 \
        '# V1 stored XI version under a core grab' grabkbd:async p38 r38 ungrabkbd 2>&1 \
        | grep -v 'core-event type=34' | sed -E 's/ dt=-?[0-9]+//'
} > xtest.log 2>&1
cat xtest.log

# Physical phase: listen while the host presses keys during the hold.
# Ends before the host's 30 s hold does, so RESULT is written in time.
(
    ./probe mon:mk:2:1k mon:ak:2:0 mon:m20:0:1 listen:25 > physical.log 2>&1; rc=$?
    # The three listeners interleave by arrival: group them, each in order.
    sort -s -k1,1 physical.log > physical-by-client.log
    if [ ! -x probe ]; then echo "fail: probe did not build (cc.log)"
    elif [ "$rc" -ne 0 ]; then echo "fail: the raw-key listener exited $rc (physical.log)"
    else echo pass; fi > RESULT
) &
sleep 2
touch LISTENING
