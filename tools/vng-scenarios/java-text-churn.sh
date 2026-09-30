# Sourced by tools/vng-shot.sh INSIDE the guest (DISPLAY=:7, cwd = artifacts).
#
# #137 step 5 -- size the tier-1 one-pixel readback. The reporter's probe
# paints its text once, which proves the fix but measures nothing. This runs
# JavaTextChurn, which repaints 32 strings in fresh colours every 10ms so
# Java2D cannot reuse a source picture: every string is a CompositeGlyphs
# with its own XRSolidSrcPict, i.e. the worst case for a per-draw readback.
#
# Run with --env YSERVER_LOOP_TELEMETRY=1 and read the per-second rollups:
#
#   grep 'loop telemetry' yserver.log   # get_image_by_site/s[... glyphsrc=N]
#
# glyphsrc/s is the readback rate. Its COUNT is hardware-independent (one per
# admitted CompositeGlyphs); its COST is not, so a cache decision still needs
# a real GPU.
set -u
: "${YS_XRENDER:=true}"
: "${YS_CHURN_SECS:=20}"
src=$(dirname "$0")/JavaTextChurn.java
[ -r "$src" ] || src=${YSERVER_REPO:?}/tools/vng-scenarios/JavaTextChurn.java
javac -d . "$src" > javac.log 2>&1 || cat javac.log >&2

# The JVM sometimes dies with SIGILL in C1-compiled code in this guest
# (hs_err in `Module.ensureNativeAccess`) — a JIT/guest-CPU interaction,
# nothing to do with the X server. Retry rather than tolerate it: a run whose
# window never mapped leaves a bare backdrop, which is easy to misread as a
# rendering failure.
attempt=0
while [ "$attempt" -lt 3 ]; do
    attempt=$((attempt + 1))
    java -Dsun.java2d.xrender="$YS_XRENDER" -cp . JavaTextChurn \
        > "java-$attempt.log" 2>&1 &
    java_pid=$!
    sleep 10
    if xwininfo -root -tree 2>/dev/null | grep -q 'Java text churn'; then
        echo "attempt $attempt: mapped" > window-mapped.txt
        cp "java-$attempt.log" java.log 2>/dev/null || true
        break
    fi
    echo "attempt $attempt: churn window never mapped" >> window-missing.txt
    kill -9 "$java_pid" 2>/dev/null || true
    rm -f hs_err_pid*.log
    sleep 2
done
sleep "$YS_CHURN_SECS"
xwininfo -root -tree > tree.txt 2>&1 || true
# Also capture through the X protocol, which is the only route available under
# `--server xorg` (on a non-composited X server the root window IS the
# framebuffer). Harmless on yserver, and it makes the two servers directly
# comparable on the same scenario.
import -window root screen.png 2>&1 | head -3 > import.log || true
